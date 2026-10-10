#!/usr/bin/env python3
"""Builds the website's game data into site-data/ (published with the game
database by scripts/publish-db.sh): one JSON per game with its cover, details
and every achievement (name, description, points, icon, and whether it can
unlock yet), plus games.json listing them all.

  scripts/build-site-data.py

Icons come from each game's `icons_from` source (Exophase or Steam) and are
cached in site-data/.icons-cache.json so reruns are quick.
"""
import html, json, re, subprocess, tomllib, urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "site-data"
UA = {"User-Agent": "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0"}
norm = lambda s: re.sub(r"[^a-z0-9]", "", s.lower())


def get(url):
    return urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=30).read().decode("utf-8", "ignore")


def exophase_icons(slug):
    page = get(f"https://www.exophase.com/game/{slug}/achievements/")
    icons = {}
    for item in page.split("<li")[1:]:
        if "award" not in item.split(">")[0]:
            continue
        img = re.search(r'src="(https://[^"]*/awards/[^"]+)"', item)
        name = re.search(r"<a[^>]*>([^<]+)</a>", item)
        if img and name:
            icons[norm(html.unescape(name.group(1)))] = img.group(1).replace("/s/", "/l/")
    return icons


def steam_icons(appid):
    page = get(f"https://steamcommunity.com/stats/{appid}/achievements/?l=english")
    icons = {}
    for row in page.split('<div class="achieveRow')[1:]:
        img = re.search(r'<img src="([^"]+)"', row)
        name = re.search(r"<h3>([^<]+)</h3>", row)
        if img and name:
            icons[norm(html.unescape(name.group(1)))] = img.group(1)
    return icons


def main():
    OUT.mkdir(exist_ok=True)
    meta = tomllib.loads((ROOT / "site" / "games.toml").read_text())
    cache_path = OUT / ".icons-cache.json"
    cache = json.loads(cache_path.read_text()) if cache_path.exists() else {}
    listing = []
    for f in sorted((ROOT / "definitions").glob("*.toml")):
        d = tomllib.loads(f.read_text())
        g = d["game"]
        source = g.get("icons_from", "")
        if source and source not in cache:
            kind, _, ref = source.partition(":")
            try:
                cache[source] = exophase_icons(ref) if kind == "exophase" else steam_icons(ref) if kind == "steam" else {}
            except Exception as e:
                print(f"{g['id']}: icons: {e}")
                cache[source] = {}
        icons = cache.get(source, {})
        achievements = []
        for a in d.get("achievement", []):
            tracked = bool(a.get("when") or a.get("steam_api") or a.get("platinum"))
            achievements.append({
                "id": a["id"],
                "name": a["name"],
                "description": a.get("description", ""),
                "points": a.get("points", 0),
                "secret": a.get("secret", False),
                "tracked": tracked,
                "icon": icons.get(norm(a.get("steam_name") or a["name"]), ""),
            })
        m = meta.get(g["id"], {})
        # When the game first appeared in the database (git), for "recently added".
        first = subprocess.run(["git", "log", "--diff-filter=A", "--format=%ct", "--", str(f)], cwd=ROOT, capture_output=True, text=True).stdout.split()
        added = int(first[-1]) if first else 0
        game = {
            "id": g["id"],
            "title": g["title"],
            "subtitle": g.get("subtitle", ""),
            "developer": g.get("developer", ""),
            "publisher": g.get("publisher", ""),
            "year": g.get("year"),
            "cover": m.get("cover", ""),
            "banner": m.get("banner", ""),
            "points": sum(a["points"] for a in achievements),
            "added": added,
            "achievements": achievements,
        }
        (OUT / f"{g['id']}.json").write_text(json.dumps(game, indent=1) + "\n")
        listing.append({k: game[k] for k in ("id", "title", "subtitle", "developer", "year", "cover", "points", "added")}
                       | {"total": len(achievements), "tracked": sum(a["tracked"] for a in achievements)})
        print(f"{g['id']}: {len(achievements)} achievements, {sum(a['tracked'] for a in achievements)} tracked, {sum(bool(a['icon']) for a in achievements)} icons")
    (OUT / "games.json").write_text(json.dumps({"games": listing}, indent=1) + "\n")
    cache_path.write_text(json.dumps(cache))


if __name__ == "__main__":
    main()
