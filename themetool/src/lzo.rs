//! LZO1X decompression (XBMC packed its skin textures with it). A direct
//! transcription of minilzo's `lzo1x_decompress`.

pub fn decompress(src: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::with_capacity(expected);
    let mut ip = 0usize;
    let byte = |i: usize| src.get(i).copied().ok_or_else(|| "truncated input".to_string());
    fn back(out: &mut Vec<u8>, dist: usize, n: usize) -> Result<(), String> {
        let start = out.len().checked_sub(dist).ok_or("match before start")?;
        for k in 0..n {
            let b = out[start + k];
            out.push(b);
        }
        Ok(())
    }
    #[derive(PartialEq)]
    enum S {
        Outer,
        FirstLiteralRun,
        Match,
        MatchDone,
        MatchNext,
    }
    let mut t: usize;
    let mut state;
    if byte(0)? > 17 {
        t = byte(0)? as usize - 17;
        ip = 1;
        if t < 4 {
            state = S::MatchNext;
        } else {
            out.extend_from_slice(src.get(ip..ip + t).ok_or("truncated literal")?);
            ip += t;
            state = S::FirstLiteralRun;
        }
    } else {
        t = 0;
        state = S::Outer;
    }
    loop {
        if state == S::Outer {
            t = byte(ip)? as usize;
            ip += 1;
            if t >= 16 {
                state = S::Match;
            } else {
                if t == 0 {
                    while byte(ip)? == 0 {
                        t += 255;
                        ip += 1;
                    }
                    t += 15 + byte(ip)? as usize;
                    ip += 1;
                }
                out.extend_from_slice(src.get(ip..ip + t + 3).ok_or("truncated literal")?);
                ip += t + 3;
                state = S::FirstLiteralRun;
            }
        }
        if state == S::FirstLiteralRun {
            t = byte(ip)? as usize;
            ip += 1;
            if t >= 16 {
                state = S::Match;
            } else {
                let dist = 1 + 0x0800 + (t >> 2) + ((byte(ip)? as usize) << 2);
                ip += 1;
                back(&mut out, dist, 3)?;
                state = S::MatchDone;
            }
        }
        if state == S::Match {
            if t >= 64 {
                let dist = 1 + ((t >> 2) & 7) + ((byte(ip)? as usize) << 3);
                ip += 1;
                back(&mut out, dist, (t >> 5) + 1)?;
            } else if t >= 32 {
                t &= 31;
                if t == 0 {
                    while byte(ip)? == 0 {
                        t += 255;
                        ip += 1;
                    }
                    t += 31 + byte(ip)? as usize;
                    ip += 1;
                }
                let dist = 1 + ((byte(ip)? as usize | (byte(ip + 1)? as usize) << 8) >> 2);
                ip += 2;
                back(&mut out, dist, t + 2)?;
            } else if t >= 16 {
                let mut dist = (t & 8) << 11;
                t &= 7;
                if t == 0 {
                    while byte(ip)? == 0 {
                        t += 255;
                        ip += 1;
                    }
                    t += 7 + byte(ip)? as usize;
                    ip += 1;
                }
                dist += (byte(ip)? as usize | (byte(ip + 1)? as usize) << 8) >> 2;
                ip += 2;
                if dist == 0 {
                    return Ok(out);
                }
                back(&mut out, dist + 0x4000, t + 2)?;
            } else {
                let dist = 1 + (t >> 2) + ((byte(ip)? as usize) << 2);
                ip += 1;
                back(&mut out, dist, 2)?;
            }
            state = S::MatchDone;
        }
        if state == S::MatchDone {
            t = (byte(ip - 2)? & 3) as usize;
            if t == 0 {
                state = S::Outer;
                continue;
            }
            state = S::MatchNext;
        }
        if state == S::MatchNext {
            out.extend_from_slice(src.get(ip..ip + t).ok_or("truncated literal")?);
            ip += t;
            t = byte(ip)? as usize;
            ip += 1;
            state = S::Match;
        }
    }
}
