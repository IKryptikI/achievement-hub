# Theme packs

The overlay ships with its own artwork and sounds for every design. A theme
pack lets you swap in files **you already have** (for the Blades design:
for example your copy of the MC360 XBMC skin). Nothing third-party is
distributed with Achievement Hub; packs live only on your machine.

## Location

`Documents/My Games/Achievement Hub/themes/blades/` (with a `theme.toml`).
The overlay loads it at startup; anything missing falls back to the
built-in look.

## Building one from the MC360 skin

```
ah-theme import-mc360 "<path>/MC360 2.1"
```

This reads the skin's `media/textures.xpr` (XBMC's LZO-packed Xbox texture
bundle), `media/bkgd_frames/` and `sounds/`, and writes:

| File | What it is |
|---|---|
| `blade-left-1..3.png`, `blade-right-1..3.png` | Folded blades (3 = nearest the open one) |
| `blade-open.png` | The open blade's strip |
| `runner-left.png`, `runner-right.png` | The silver edges |
| `body-{green,orange,blue,purple}.png` | Blade colours (1 px wide gradients) |
| `glass-{top,bottom}-{left,middle,right}.png` | Frosted header/footer bands |
| `sheen-{left,middle,right}.png` | The body's glass overlay |
| `focus.png`, `nofocus.png` | Selection bar |
| `card.png`, `card-focus.png` | Gamer-card frame (gold title bar), used for cards |
| `glyph-{a,b,x,y}.png` | Controller buttons |
| `swirl/NN.png` | Animated background frames |
| `font.ttf` | The skin's font (used for all Blades text) |
| `sounds/{move,select,back,switch,open,close,notify}.wav` | Interface sounds |

Pieces are placed on a 720x576 layout taken from the dashboard and scaled
to the screen.

## Sounds

Your own `Documents/My Games/Achievement Hub/sounds/<name>.wav` always
wins; then the theme pack's (while its design is on); then the built-in
ones. `sounds=off` in `achievement-hub.ini` mutes them.
