# Resident Evil 2 (classic, GOG) — memory map

Build: LeonU.exe, PE timestamp 0x370c3909 (1999-04-08), image size 0x5ab000, base 0x400000.
(ClaireU.exe not yet checked — likely a different build/addresses.)

## Confirmed
- Inventory: array of 4-byte slots starting at 0x999334 (LeonU.exe).
  Slot layout: [u8 item_id, u8 count, u8 ?, u8 ?]. Inventory order = slot order.
  Address is stable across game sessions (verified on two runs).
  11 slots: 0x999334 + 4*i, i = 0..9 normal slots; 0x99935c = special slot (Lighter).
  Item IDs: 0x01 Knife, 0x02 Handgun (count = rounds loaded), 0x14 Handgun Bullets,
  0x1E Ink Ribbon, 0x2F Lighter, 0x33 Red Jewel.

- Player HP: s16 at 0x994646 (LeonU.exe). Max 200. Verified 2026-10-05: dropped on
  each hit across two sessions; 73 -> 200 on First Aid Spray. Status at 73 = Caution.

## Not health (display/draw data — ignore)
- 0x6949ac, 0x95c702 / 0x95d302 (double-buffered draw lists), 0x688cxx (draw node list).
  The exe data area 0x5xx000–0x9xxxxx contains lots of per-frame draw data.

## Open
- Game clear / rank / save count: not yet searched.
