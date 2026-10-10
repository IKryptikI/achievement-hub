#!/usr/bin/env bash
# Build the overlay + launcher and install them, with all definitions, into a
# game folder:  scripts/deploy.sh "/path/to/game folder"
#
# Safe while the game is running: files are swapped in as new files, so a
# running game keeps the copy it already loaded and picks up the new one on
# its next launch.
set -euo pipefail
GAME=${1:?usage: deploy.sh GAME_DIR}
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/target/i686-pc-windows-gnu/release"

cargo build --release -p overlay -p launcher --target i686-pc-windows-gnu --manifest-path "$ROOT/Cargo.toml"

swap() { cp "$1" "$2.new" && mv -f "$2.new" "$2"; }
swap "$OUT/overlay.dll" "$GAME/overlay.dll"
swap "$OUT/ah-launch.exe" "$GAME/ah-launch.exe"
mkdir -p "$GAME/games"
for f in "$ROOT"/definitions/*.toml; do swap "$f" "$GAME/games/$(basename "$f")"; done
[[ -f "$GAME/achievement-hub.ini" ]] || echo "note: no achievement-hub.ini in $GAME (ah-launch needs launch= and inject=)"
echo "deployed to $GAME"
