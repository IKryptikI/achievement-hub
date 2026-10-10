#!/usr/bin/env python3
"""Start a game's definition from an existing achievement/trophy list.

Lists come from Exophase, which indexes Xbox 360/One/Series, PlayStation,
Steam, GOG and retro platforms (its public search API plus the list page).
The definition gets the names, descriptions, points and an icon source;
the memory conditions (`when`) are still ours to map.

  scripts/import-achievements.py search "Saw"
  scripts/import-achievements.py import saw-xbox-360 --exe "/path/to/Game.exe"
      [--id saw] [--title "Saw"] [--subtitle ...] [--force]

`import` takes an Exophase slug or URL and writes definitions/<id>.toml.
"""
import argparse, html, json, re, struct, sys, urllib.parse, urllib.request
from pathlib import Path

UA = "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0"
ROOT = Path(__file__).resolve().parent.parent
SUFFIXES = ["xbox-360", "xbox", "xbox-one", "psn", "steam", "gog", "retro", ""]


def get(url):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.read().decode("utf-8", "ignore")


def slugify(name):
    return re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")


def exists(slug):
    try:
        t = get(f"https://www.exophase.com/game/{slug}/achievements/")
        m = re.search(r"<title>([^<]*)", t)
        return m.group(1).split(" - Exophase")[0] if m else slug
    except Exception:
        return None


def search(name):
    found = []
    try:
        d = json.loads(get("https://api.exophase.com/public/archive/games?q=" + urllib.parse.quote(name)))
        for g in (d.get("games") or {}).get("list") or []:
            slug = g["endpoint_awards"].rstrip("/").split("/")[-2]
            found.append((g["title"], g["platforms"][0]["name"], g["total_awards"], g["total_points"], slug))
    except Exception as e:
        print(f"(search API: {e})", file=sys.stderr)
    # The API is weak on short names: also try the obvious page names.
    base = slugify(name)
    guessed = []
    for suf in SUFFIXES:
        slug = f"{base}-{suf}" if suf else base
        if any(f[4] == slug for f in found):
            continue
        title = exists(slug)
        if title:
            guessed.append((title, suf or "?", "?", "?", slug))
    found = guessed + found
    for t, plat, n, pts, slug in found[:25]:
        print(f"{slug:45} {plat:12} {str(n):>4} {str(pts):>6}  {t}")


def parse(slug):
    t = get(f"https://www.exophase.com/game/{slug}/achievements/")
    title = re.search(r"<title>([^<]*)", t).group(1).split(" Achievements")[0].split(" Trophies")[0]
    rows = []
    for it in re.findall(r'(<li[^>]*class="[^"]*award[^"]*"[^>]*>.*?</li>)', t, re.S):
        a = re.search(r"<a[^>]*>([^<]+)</a>", it)
        desc = re.search(r'class="[^"]*award-description[^"]*"[^>]*>\s*<p>(.*?)</p>', it, re.S)
        pts = re.search(r'class="[^"]*award-points[^"]*"[^>]*>.*?(\d+)', it, re.S)
        if not a:
            continue
        rows.append({
            "name": html.unescape(a.group(1).strip()),
            "desc": html.unescape(re.sub(r"<[^>]+>", "", desc.group(1)).strip()) if desc else "",
            "pts": int(pts.group(1)) if pts else 0,
            "secret": "secret" in it.split(">")[0],
        })
    return title, rows


def exe_build(path):
    d = open(path, "rb").read(4096)
    pe = struct.unpack_from("<I", d, 0x3C)[0]
    machine = struct.unpack_from("<H", d, pe + 4)[0]
    return struct.unpack_from("<I", d, pe + 8)[0], struct.unpack_from("<I", d, pe + 0x50)[0], machine == 0x8664


def toml_str(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def import_(slug, exe, gid, title, subtitle, force):
    slug = slug.rstrip("/").split("/game/")[-1].split("/")[0]
    src_title, rows = parse(slug)
    if not rows:
        sys.exit(f"no achievements found on {slug}")
    ts, size, x64 = exe_build(exe)
    if x64:
        print("note: 64-bit exe; the overlay doesn't support 64-bit games yet", file=sys.stderr)
    title = title or src_title
    gid = gid or slugify(title)
    out = ROOT / "definitions" / f"{gid}.toml"
    if out.exists() and not force:
        sys.exit(f"{out} exists (use --force)")
    total = sum(r["pts"] for r in rows)
    lines = [
        f"# {title}: achievements imported from Exophase ({slug}), {len(rows)} total, {total} points.",
        "# Achievements without `when` are listed but can't unlock yet (their",
        "# memory conditions haven't been found).",
        "",
        "[game]",
        f"id = {toml_str(gid)}",
        f"title = {toml_str(title)}",
    ]
    if subtitle:
        lines.append(f"subtitle = {toml_str(subtitle)}")
    lines += [
        f'icons_from = "exophase:{slug}"',
        "",
        "[[build]]",
        f"exe = {toml_str(Path(exe).name)}",
        f"timestamp = {ts:#010x}",
        f"image_size = {size:#x}",
        "",
    ]
    seen = set()
    for r in rows:
        i = slugify(r["name"]).replace("-", "_") or "achievement"
        while i in seen:
            i += "_2"
        seen.add(i)
        plat = re.search(r"\ball (other )?(achievements|trophies)\b", r["desc"], re.I) is not None
        lines += ["[[achievement]]", f"id = {toml_str(i)}", f"name = {toml_str(r['name'])}", f"description = {toml_str(r['desc'])}", f"points = {r['pts']}"]
        if r["secret"]:
            lines.append("secret = true")
        if plat:
            lines.append("platinum = true")
        lines.append("")
    out.write_text("\n".join(lines))
    print(f"wrote {out}: {len(rows)} achievements, {total} points, build {ts:#010x}/{size:#x}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("search")
    s.add_argument("name")
    i = sub.add_parser("import")
    i.add_argument("slug")
    i.add_argument("--exe", required=True)
    i.add_argument("--id")
    i.add_argument("--title")
    i.add_argument("--subtitle")
    i.add_argument("--force", action="store_true")
    a = ap.parse_args()
    if a.cmd == "search":
        search(a.name)
    else:
        import_(a.slug, a.exe, a.id, a.title, a.subtitle, a.force)


if __name__ == "__main__":
    main()
