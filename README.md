# Achievement Hub

Achievements for PC games that never had them, shown in-game like Steam,
GFWL or RetroAchievements.

**Website, downloads and accounts:** https://achievementhub.pages.dev

## Download

Grab the latest release from the [Releases page](https://github.com/IKryptikI/achievement-hub/releases/latest):

- **Windows:** `AchievementHub-windows.zip`. Unzip anywhere and run `AchievementHub.exe` (keep the `payload` folder next to it).
- **Linux:** `AchievementHub-x86_64.AppImage`. `chmod +x` it and run. Games run through Proton, Wine, Heroic, Lutris or Bottles.

## How it works

1. **ah-launch.exe** starts the game (configured by `achievement-hub.ini` in
   the game folder) and loads `overlay.dll` into it.
2. **overlay.dll** hooks the game's renderer to draw pop-ups and the
   achievements panel, and checks achievement rules against the game's
   memory ~10 times a second.
3. Achievements come from `games/*.toml` next to the DLL (copied from
   `definitions/`); unlocks are saved to `achievements.txt`.

The same Windows binaries run natively on Windows and under Proton/Wine.

## Layout

| Path | What |
|---|---|
| `hub/` | Desktop app: detect installed games, anti-cheat blacklist, library, install the overlay, request games |
| `common/` | Code shared by several parts (anti-cheat detection) |
| `overlay/` | In-game overlay DLL (hooks D3D9/D3D11/OpenGL via a patched hudhook) |
| `launcher/` | `ah-launch.exe`: starts a game and injects the overlay |
| `tracker/` | Linux/Windows tools for finding game memory: `probe`, `scan` |
| `definitions/` | Achievement definition files, one per game build (format: `definitions/README.md`) |
| `games/` | Research notes: memory maps per game |
| `scripts/deploy.sh` | Build and install overlay, launcher and definitions into a game folder |
| `vendor/hudhook/` | hudhook 0.9.3 (MIT) with fixes, see the top of `src/hooks/dx9.rs` and `Pipeline::new` |

## Building (from Linux)

```sh
# Tools (native)
cargo build --release -p tracker
# Desktop hub app (native)
cargo build --release -p hub     # `hub --scan` prints detected games and their status
# Overlay + launcher (32-bit Windows; needs mingw32-gcc / mingw32-gcc-c++)
cargo build --release -p overlay -p launcher --target i686-pc-windows-gnu
```

## Overlay controls

Shift+Tab, the Xbox button or Back+Start opens the achievements panel; Esc or B also
close it. Configurable in `achievement-hub.ini` (`hotkey=`, `pad_hotkey=`).

## License

GPL-3.0. See `LICENSE`. `vendor/hudhook` is MIT (its own license file).
Achievement names and descriptions belong to their games' publishers.
