//! Reads XBMC's packed skin textures (`.xpr` bundles): a table of named
//! entries, each LZO-compressed, each an Xbox Direct3D texture (header plus
//! swizzled or DXT pixels).

use crate::lzo;
use image::RgbaImage;

pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
}

fn u32le(d: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]])
}

/// Every entry in a bundle, decompressed.
pub fn entries(bundle: &[u8]) -> Result<Vec<Entry>, String> {
    if bundle.len() < 12 || &bundle[..3] != b"XPR" {
        return Err("not an XBMC texture bundle".into());
    }
    let header = u32le(bundle, 8) as usize;
    let mut out = Vec::new();
    let mut p = 12;
    while p + 128 <= header.min(bundle.len()) {
        let raw_name = &bundle[p..p + 116];
        let name = String::from_utf8_lossy(&raw_name[..raw_name.iter().position(|&b| b == 0).unwrap_or(116)]).into_owned();
        let (off, unpacked, packed) = (u32le(bundle, p + 116) as usize, u32le(bundle, p + 120) as usize, u32le(bundle, p + 124) as usize);
        p += 128;
        if name.is_empty() {
            continue;
        }
        let Some(raw) = bundle.get(off..off + packed) else { continue };
        let data = if packed < unpacked { lzo::decompress(raw, unpacked)? } else { raw.to_vec() };
        out.push(Entry { name, data });
    }
    Ok(out)
}

/// Xbox swizzle: x and y bits interleaved (x first) while both have bits.
fn morton(x: u32, y: u32, w: u32, h: u32) -> usize {
    let (mut idx, mut bit, mut sx, mut sy) = (0usize, 0, 1u32, 1u32);
    while sx < w || sy < h {
        if sx < w {
            idx |= (((x / sx) & 1) as usize) << bit;
            bit += 1;
            sx <<= 1;
        }
        if sy < h {
            idx |= (((y / sy) & 1) as usize) << bit;
            bit += 1;
            sy <<= 1;
        }
    }
    idx
}

fn rgb565(c: u16) -> [u8; 3] {
    [((c >> 11) as u32 * 255 / 31) as u8, (((c >> 5) & 63) as u32 * 255 / 63) as u8, ((c & 31) as u32 * 255 / 31) as u8]
}

/// One 4x4 DXT block (1, 3 or 5) as 16 RGBA pixels.
fn dxt_block(b: &[u8], kind: u8) -> [[u8; 4]; 16] {
    let (cb, alpha): (&[u8], [u8; 16]) = match kind {
        1 => (&b[0..8], [255; 16]),
        3 => {
            let a = u64::from_le_bytes(b[0..8].try_into().unwrap());
            (&b[8..16], std::array::from_fn(|i| (((a >> (4 * i)) & 15) * 17) as u8))
        }
        _ => {
            let (a0, a1) = (b[0] as u32, b[1] as u32);
            let bits = u64::from_le_bytes([b[2], b[3], b[4], b[5], b[6], b[7], 0, 0]);
            let pal: [u32; 8] = if a0 > a1 {
                std::array::from_fn(|i| match i {
                    0 => a0,
                    1 => a1,
                    k => ((8 - k as u32) * a0 + (k as u32 - 1) * a1) / 7,
                })
            } else {
                std::array::from_fn(|i| match i {
                    0 => a0,
                    1 => a1,
                    6 => 0,
                    7 => 255,
                    k => ((6 - k as u32) * a0 + (k as u32 - 1) * a1) / 5,
                })
            };
            (&b[8..16], std::array::from_fn(|i| pal[((bits >> (3 * i)) & 7) as usize] as u8))
        }
    };
    let (c0, c1) = (u16::from_le_bytes([cb[0], cb[1]]), u16::from_le_bytes([cb[2], cb[3]]));
    let (r0, r1) = (rgb565(c0), rgb565(c1));
    let four = c0 > c1 || kind != 1;
    let cols: [[u8; 3]; 4] = [
        r0,
        r1,
        std::array::from_fn(|i| if four { ((2 * r0[i] as u32 + r1[i] as u32) / 3) as u8 } else { ((r0[i] as u32 + r1[i] as u32) / 2) as u8 }),
        std::array::from_fn(|i| if four { ((r0[i] as u32 + 2 * r1[i] as u32) / 3) as u8 } else { 0 }),
    ];
    let idx = u32::from_le_bytes([cb[4], cb[5], cb[6], cb[7]]);
    std::array::from_fn(|i| {
        let k = ((idx >> (2 * i)) & 3) as usize;
        let a = if !four && k == 3 { 0 } else { alpha[i] };
        [cols[k][0], cols[k][1], cols[k][2], a]
    })
}

/// Decodes an entry's texture (first frame), or None for formats we don't
/// handle.
pub fn decode(d: &[u8]) -> Option<RgbaImage> {
    // The texture's Format dword sits in the 128-byte header; its low byte
    // is 0x29 (DMA channel 1, 2D).
    let (fmt, w, h) = (0..120).find_map(|i| {
        if d.get(i) != Some(&0x29) || i + 4 > d.len() {
            return None;
        }
        let v = u32le(d, i);
        let fmt = (v >> 8) & 0xff;
        matches!(fmt, 0x06 | 0x07 | 0x0b | 0x0c | 0x0e | 0x0f | 0x12 | 0x1e).then(|| (fmt, 1u32 << ((v >> 20) & 15), 1u32 << ((v >> 24) & 15)))
    })?;
    let px = d.get(128..)?;
    let mut img = RgbaImage::new(w, h);
    match fmt {
        0x06 | 0x07 | 0x12 | 0x1e => {
            let linear = matches!(fmt, 0x12 | 0x1e);
            let opaque = matches!(fmt, 0x07 | 0x1e);
            for y in 0..h {
                for x in 0..w {
                    let s = if linear { (y * w + x) as usize } else { morton(x, y, w, h) } * 4;
                    let p = px.get(s..s + 4)?;
                    img.put_pixel(x, y, image::Rgba([p[2], p[1], p[0], if opaque { 255 } else { p[3] }]));
                }
            }
        }
        0x0b => {
            let n = (w * h) as usize;
            let pal = px.get(n..n + 1024)?;
            for y in 0..h {
                for x in 0..w {
                    let i = *px.get(morton(x, y, w, h))? as usize * 4;
                    img.put_pixel(x, y, image::Rgba([pal[i + 2], pal[i + 1], pal[i], pal[i + 3]]));
                }
            }
        }
        _ => {
            let kind = match fmt {
                0x0c => 1,
                0x0e => 3,
                _ => 5,
            };
            let bs = if kind == 1 { 8 } else { 16 };
            let mut p = 0;
            for by in 0..(h / 4).max(1) {
                for bx in 0..(w / 4).max(1) {
                    let block = dxt_block(px.get(p..p + bs)?, kind);
                    p += bs;
                    for (i, c) in block.iter().enumerate() {
                        let (x, y) = (bx * 4 + i as u32 % 4, by * 4 + i as u32 / 4);
                        if x < w && y < h {
                            img.put_pixel(x, y, image::Rgba(*c));
                        }
                    }
                }
            }
        }
    }
    Some(img)
}
