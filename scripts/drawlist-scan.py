#!/usr/bin/env python3
"""Flags code that calls a draw helper while holding a window draw list
(`let dl = ...get_window_draw_list()`): imgui-rs allows only one at a time
and panics (crashing the game) otherwise."""
import re, sys, glob
helpers = re.compile(r'\b(gamerscore|shadow_text|progress|button_hint|points_pill|sideways_text|get_window_draw_list|empty|fade|child|button_hint_width|rounded_gradient|tooltip_text|gamer_card|fill_slices|radial_glow|with_clip|glyph)\(|(?<![_a-z])icon\(ui')
bad = 0
for p in sys.argv[1:] or glob.glob('overlay/src/**/*.rs', recursive=True):
    lines = open(p).read().split('\n')
    for i, l in enumerate(lines):
        if not re.search(r'^\s*let dl = ', l):
            continue
        depth = 0
        for j in range(i + 1, len(lines)):
            s = lines[j]
            if 'drop(dl)' in s:
                break
            if helpers.search(s):
                print(f"{p}:{j+1}: {s.strip()}  (draw list held since line {i+1})")
                bad += 1
            depth += s.count('{') - s.count('}')
            if depth < 0:
                break
sys.exit(1 if bad else 0)
