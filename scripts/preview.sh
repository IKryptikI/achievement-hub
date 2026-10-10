#!/usr/bin/env bash
# Renders the overlay panel to PNG screenshots without a game, using the
# ah-preview.exe test app under Wine (your profile is only read, never written).
#
#   scripts/preview.sh OUT_DIR [skin:blade ...]
#   e.g. scripts/preview.sh /tmp/shots blades:now blades:games metro:home
#
# Needs wine and ffmpeg. The Wine prefix lives in OUT_DIR/pfx.
set -euo pipefail
out=${1:?usage: preview.sh OUT_DIR [skin:blade ...]}
shift
shots=("${@:-blades:now}")
root=$(cd "$(dirname "$0")/.." && pwd)

cargo build --release -p overlay -p preview --target i686-pc-windows-gnu >/dev/null
mkdir -p "$out/run/games"
cp "$root"/target/i686-pc-windows-gnu/release/{overlay.dll,ah-preview.exe} "$out/run/"
cp "$root"/definitions/*.toml "$out/run/games/"
export WINEPREFIX="$out/pfx" WINEDEBUG=-all
[ -d "$WINEPREFIX" ] || wineboot -i >/dev/null 2>&1

cd "$out/run"
for shot in "${shots[@]}"; do
    skin=${shot%%:*}
    blade=${shot#*:}
    name="$skin-$blade"
    rm -f preview.bmp
    if AH_SKIN=$skin AH_BLADE=$blade AH_OUT=preview.bmp timeout 40 wine ah-preview.exe 2>/dev/null && [ -f preview.bmp ]; then
        ffmpeg -v error -y -i preview.bmp "$out/$name.png"
        echo "$out/$name.png"
    else
        echo "$name: failed (see $out/run/overlay.log, preview.log)" >&2
    fi
done
