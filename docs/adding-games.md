# Adding a game

A game is supported by one definition file in `definitions/`. The overlay
loads it when the running exe matches its build, and only then hooks
anything.

## 1. Check it's safe

No anti-cheat (EAC, BattlEye, VAC, etc.). The launcher and overlay refuse
games that have it, and so should we: the point is never to risk a ban.

## 2. Identify the exact build

Definitions match on the exe's PE timestamp and image size (so a patched
exe never reads the wrong memory). Get them with:

```
python3 -c "import struct,sys;d=open(sys.argv[1],'rb').read();p=struct.unpack_from('<I',d,0x3c)[0];print(hex(struct.unpack_from('<I',d,p+8)[0]),hex(struct.unpack_from('<I',d,p+0x50)[0]))" Game.exe
```

The overlay is 32-bit with Direct3D 9 hooks today. 64-bit or DX11 games
need the engine work first.

## 3. Write the definition

If the game had achievements or trophies anywhere (Xbox, PlayStation,
Steam, GOG...), start from that list:

```
scripts/import-achievements.py search "Game name"
scripts/import-achievements.py import <slug> --exe "/path/to/Game.exe" [--id x] [--title "..."]
```

That writes `definitions/<id>.toml` with every achievement's name,
description and points, plus `icons_from` so players get the official
icons. Prefer the console list when it has points (Steam lists don't).
Otherwise write it by hand:


`definitions/<game>.toml`:

```toml
[game]
id = "oblivion-gog"           # stable, unique
title = "The Elder Scrolls IV: Oblivion"
developer = "Bethesda Game Studios"   # optional; the store's details win
year = 2006

[[build]]
exe = "Oblivion.exe"
timestamp = 0x...
image_size = 0x...

[[achievement]]
id = "kvatch"
name = "..."
description = "..."
points = 20
category = "Main Quest"
# `when` conditions read game memory; without them the achievement is
# listed but can't unlock yet.
[[achievement.when]]
read = "u32"
address = 0x...
ge = 1
```

Points should total 1000. For games that had achievements on Xbox or
PlayStation, import that list (names, descriptions, points) as a starting
point. Format details: `definitions/README.md`.

## 4. Find the memory values

Easiest: **record mode**. While a game has achievements with no conditions,
the overlay watches the game's static memory by itself. Play; right after
you earn one, open the overlay, highlight it on Now Playing and press A
(Enter). That unlocks it for you and saves what changed to
`<profile>/mapping/<game>/<achievement>.json`. Usually a few possibilities
are left: reload a save from just before, earn it again and press A again;
only changes that happened the same way both times are kept. Once one is
left it's "Mapped" and unlocks by itself from then on. Then

```
scripts/merge-mappings.py <game id>
```

copies everything mapped into `definitions/<game id>.toml` so everyone gets
it. (`record_mode = false` in achievement-hub.ini turns the watching off.)
Record mode sees the exe's own writable memory, where older games keep
their progress; newer engines keep it on the heap and need an `[engine]`
block or, if they had Steam achievements, `steam_api` names.

By hand:

Each condition needs an address (or pointer chain) that changes when the
achievement's event happens. Find them with a memory scanner while
playing (search, change the value in game, narrow down), then note them
in `games/<game>.md` like `games/re2-classic-gog.md`.

## 5. Try it

```
scripts/deploy.sh "<game folder>"
```

copies the overlay, launcher and definitions in. Point the store/launcher
at `ah-launch.exe` (Heroic: target exe; others: see `achievement-hub.ini`).
`overlay.log` in the game folder says whether the build matched.
