# Achievement definitions

One TOML file per game build. The overlay loads every `*.toml` in the
`games/` folder next to `overlay.dll` and uses the file whose `[[build]]`
matches the running exe.

```toml
[game]
id = "re2-classic-gog"          # stable id; unlocks are stored under it
title = "Resident Evil 2 (Leon)"

[[build]]                        # one or more; matched by PE header
exe = "LeonU.exe"                # informational
timestamp = 0x370c3909           # PE TimeDateStamp (`probe` prints it)
image_size = 0x5ab000            # PE SizeOfImage

[[achievement]]
id = "red_jewel"                 # stable; never reuse or rename
name = "Crimson Gem"
description = "Find a Red Jewel."
when = [ { read = "u8", address = 0x999334, stride = 4, count = 11, eq = 0x33 } ]
```

An achievement unlocks the first time **every** condition in `when` holds
at the same check (about 10 checks a second).

## Conditions

| Key | Meaning |
|---|---|
| `read` | `u8` `i8` `u16` `i16` `u32` `i32` (little-endian) |
| `address` | absolute address in the game process |
| `previous` | `true` = use the value from the previous check (detects changes) |
| `mask` | only these bits count, e.g. `0x20` (flags packed into a byte; record mode writes these) |
| `relative` | `true` = `address` is an offset from where the game exe is loaded (exes that move between launches; record mode writes these) |
| `stride`, `count` | check `count` values `stride` bytes apart; holds if **any** matches |
| `eq` `ne` `lt` `le` `gt` `ge` | compare with a number |
| `between` | `[lo, hi]`, inclusive |

Several comparisons in one condition must all hold
(`{ ..., ge = 1, le = 119 }` is the same as `between = [1, 119]`).

An unreadable address makes its condition false; it can't crash the game.
