//! Sixel: a picture as bands six pixels tall, each pixel one of a palette
//! of colour registers (DEC's VT340 format; foot, WezTerm, mlterm,
//! Konsole, xterm built with it). Pure: RGB pixels in, bytes out.
//!
//! The palette is a median cut of the picture to [`MAX_COLORS`] (album
//! covers survive it well), each pixel mapped to its nearest entry through
//! a 15-bit lookup. Every pixel gets a colour, so the picture is opaque.

use std::fmt::Write as _;

/// Colour registers used at most (what foot, WezTerm, mlterm and Konsole
/// offer; xterm needs `numColorRegisters` raised past its 16).
pub const MAX_COLORS: usize = 256;

/// Pixels the palette is chosen from at most (a sample of bigger pictures).
const SAMPLE: usize = 16 * 1024;

/// `w × h` pixels, row by row, as one sixel image (DCS … ST). The cursor
/// is left on the image's last band (no trailing graphics newline, so
/// nothing can scroll).
pub fn encode(pixels: &[[u8; 3]], w: usize, h: usize) -> Vec<u8> {
    assert_eq!(pixels.len(), w * h, "pixels for {w}×{h}");
    let palette = median_cut(pixels, MAX_COLORS);
    let index = map(pixels, &palette);
    // `P2 = 1`: pixels left unset stay as they were (there are none).
    let mut text = format!("\x1bP0;1;0q\"1;1;{w};{h}");
    for (i, c) in palette.iter().enumerate() {
        let pct = |v: u8| (u32::from(v) * 100 + 127) / 255;
        let _ = write!(text, "#{i};2;{};{};{}", pct(c[0]), pct(c[1]), pct(c[2]));
    }
    let mut out = text.into_bytes();
    // One band: per colour, the six bits of each column.
    let mut bits = vec![0u8; palette.len() * w];
    let mut used = vec![false; palette.len()];
    for (b, top) in (0..h).step_by(6).enumerate() {
        if b > 0 {
            out.push(b'-');
        }
        let rows = (h - top).min(6);
        for r in 0..rows {
            let row = &index[(top + r) * w..(top + r + 1) * w];
            for (x, &c) in row.iter().enumerate() {
                let c = usize::from(c);
                bits[c * w + x] |= 1 << r;
                used[c] = true;
            }
        }
        let mut first = true;
        for c in 0..palette.len() {
            if !std::mem::take(&mut used[c]) {
                continue;
            }
            if !first {
                // Back to the band's start for the next colour.
                out.push(b'$');
            }
            first = false;
            out.extend_from_slice(format!("#{c}").as_bytes());
            let columns = &mut bits[c * w..(c + 1) * w];
            let end = columns.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
            run_length(&columns[..end], &mut out);
            columns.fill(0);
        }
    }
    out.extend_from_slice(b"\x1b\\");
    out
}

/// Sixel characters for `columns` (six bits each), runs of four or more
/// as `!n`.
fn run_length(columns: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < columns.len() {
        let bits = columns[i];
        let n = columns[i..].iter().take_while(|&&b| b == bits).count();
        let ch = 63 + bits;
        if n >= 4 {
            out.extend_from_slice(format!("!{n}").as_bytes());
            out.push(ch);
        } else {
            out.extend(std::iter::repeat_n(ch, n));
        }
        i += n;
    }
}

/// Up to `n` colours standing for `pixels`: split the box with the widest
/// channel range at its median until there are `n` (or nothing left to
/// split), then average each box.
fn median_cut(pixels: &[[u8; 3]], n: usize) -> Vec<[u8; 3]> {
    let step = (pixels.len() / SAMPLE).max(1);
    let mut boxes: Vec<Vec<[u8; 3]>> = vec![pixels.iter().step_by(step).copied().collect()];
    if boxes[0].is_empty() {
        return vec![[0, 0, 0]];
    }
    while boxes.len() < n {
        let widest = boxes
            .iter()
            .enumerate()
            .map(|(i, b)| (i, widest_channel(b)))
            .max_by_key(|&(_, (_, range))| range);
        let Some((i, (channel, range))) = widest else {
            break;
        };
        if range == 0 {
            break;
        }
        let mut b = boxes.swap_remove(i);
        b.sort_unstable_by_key(|p| p[channel]);
        let upper = b.split_off(b.len() / 2);
        boxes.push(b);
        boxes.push(upper);
    }
    boxes
        .iter()
        .map(|b| {
            let mut sum = [0u64; 3];
            for p in b {
                for (s, &v) in sum.iter_mut().zip(p) {
                    *s += u64::from(v);
                }
            }
            let n = b.len() as u64;
            sum.map(|s| ((s + n / 2) / n) as u8)
        })
        .collect()
}

/// The channel whose values spread widest in `b`, and how wide.
fn widest_channel(b: &[[u8; 3]]) -> (usize, u8) {
    (0..3)
        .map(|c| {
            let (lo, hi) = b
                .iter()
                .fold((u8::MAX, 0), |(lo, hi), p| (lo.min(p[c]), hi.max(p[c])));
            (c, hi.saturating_sub(lo))
        })
        .max_by_key(|&(_, range)| range)
        .unwrap_or((0, 0))
}

/// Each pixel's nearest palette entry, looked up by its 15-bit colour.
fn map(pixels: &[[u8; 3]], palette: &[[u8; 3]]) -> Vec<u8> {
    const UNSET: u16 = u16::MAX;
    let mut memo = vec![UNSET; 1 << 15];
    pixels
        .iter()
        .map(|p| {
            let key = (usize::from(p[0] >> 3) << 10)
                | (usize::from(p[1] >> 3) << 5)
                | usize::from(p[2] >> 3);
            if memo[key] == UNSET {
                memo[key] = nearest(*p, palette) as u16;
            }
            memo[key] as u8
        })
        .collect()
}

fn nearest(p: [u8; 3], palette: &[[u8; 3]]) -> usize {
    let d = |c: &[u8; 3]| {
        (0..3)
            .map(|i| {
                let e = i32::from(p[i]) - i32::from(c[i]);
                e * e
            })
            .sum::<i32>()
    };
    palette
        .iter()
        .enumerate()
        .min_by_key(|(_, c)| d(c))
        .map_or(0, |(i, _)| i)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Decode `bytes` back to pixels (a minimal sixel reader: what these
    /// tests need, run-lengths, `$`, `-`, `#n` and `#n;2;r;g;b`).
    pub(crate) fn decode(bytes: &[u8]) -> (usize, usize, Vec<Option<[u8; 3]>>) {
        let text = std::str::from_utf8(bytes).unwrap();
        let body = text
            .strip_prefix("\x1bP0;1;0q\"1;1;")
            .and_then(|t| t.strip_suffix("\x1b\\"))
            .expect("DCS … ST with raster attributes");
        let mut chars = body.chars().peekable();
        fn number(chars: &mut std::iter::Peekable<std::str::Chars>) -> usize {
            let mut n = 0usize;
            while let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
                n = n * 10 + d as usize;
                chars.next();
            }
            n
        }
        let w = number(&mut chars);
        assert_eq!(chars.next(), Some(';'));
        let h = number(&mut chars);
        let mut out = vec![None; w * h];
        let mut regs = vec![[0u8; 3]; 256];
        let (mut x, mut band, mut colour) = (0, 0, 0);
        let pct = |v: usize| ((v * 255 + 50) / 100) as u8;
        while let Some(c) = chars.next() {
            match c {
                '#' => {
                    colour = number(&mut chars);
                    if chars.peek() == Some(&';') {
                        chars.next();
                        assert_eq!(number(&mut chars), 2, "RGB");
                        let mut rgb = [0; 3];
                        for v in &mut rgb {
                            assert_eq!(chars.next(), Some(';'));
                            *v = pct(number(&mut chars));
                        }
                        regs[colour] = rgb;
                    }
                }
                '$' => x = 0,
                '-' => {
                    x = 0;
                    band += 1;
                }
                '!' | '?'..='~' => {
                    let (n, ch) = if c == '!' {
                        let n = number(&mut chars);
                        (n, chars.next().unwrap())
                    } else {
                        (1, c)
                    };
                    let bits = ch as u8 - 63;
                    for _ in 0..n {
                        for r in 0..6 {
                            let y = band * 6 + r;
                            if bits & (1 << r) != 0 {
                                assert!(x < w && y < h, "pixel ({x}, {y}) outside {w}×{h}");
                                out[y * w + x] = Some(regs[colour]);
                            }
                        }
                        x += 1;
                    }
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        (w, h, out)
    }

    fn close(a: [u8; 3], b: [u8; 3], tolerance: u8) -> bool {
        (0..3).all(|i| a[i].abs_diff(b[i]) <= tolerance)
    }

    #[test]
    fn a_picture_round_trips_through_its_bands() {
        // 13 × 15: a partial last band, colours in stripes and a corner.
        let (w, h) = (13, 15);
        let pixel = |x: usize, y: usize| match (x, y) {
            (0..=2, 0..=2) => [255, 255, 255],
            _ if y % 4 < 2 => [200, 40, 30],
            _ => [20, 60, 180],
        };
        let pixels: Vec<_> = (0..w * h).map(|i| pixel(i % w, i / w)).collect();
        let bytes = encode(&pixels, w, h);
        let (dw, dh, out) = decode(&bytes);
        assert_eq!((dw, dh), (w, h));
        for (i, got) in out.iter().enumerate() {
            let got = got.unwrap_or_else(|| panic!("pixel {i} unset"));
            assert!(close(got, pixels[i], 3), "{i}: {got:?} vs {:?}", pixels[i]);
        }
        // Three bands: two graphics newlines, none after the last.
        assert_eq!(bytes.iter().filter(|&&b| b == b'-').count(), 2);
    }

    #[test]
    fn many_colours_are_cut_to_the_palette_and_stay_close() {
        let (w, h) = (64, 48);
        let pixels: Vec<[u8; 3]> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                [(x * 4) as u8, (y * 5) as u8, ((x + y) * 2) as u8]
            })
            .collect();
        let bytes = encode(&pixels, w, h);
        let registers = median_cut(&pixels, MAX_COLORS).len();
        assert!(registers <= MAX_COLORS && registers > 128, "{registers}");
        let (_, _, out) = decode(&bytes);
        let worst = out
            .iter()
            .zip(&pixels)
            .map(|(got, want)| {
                let got = got.unwrap();
                (0..3).map(|i| got[i].abs_diff(want[i])).max().unwrap()
            })
            .max()
            .unwrap();
        assert!(worst <= 24, "worst channel error {worst}");
    }

    #[test]
    fn runs_are_compressed() {
        let pixels = vec![[9, 9, 9]; 100 * 6];
        let text = String::from_utf8(encode(&pixels, 100, 6)).unwrap();
        // One colour, one band: `#0!100~`.
        assert!(text.ends_with("#0!100~\x1b\\"), "{text:?}");
        let mut out = Vec::new();
        run_length(&[1, 1, 1, 2, 2, 2, 2, 0], &mut out);
        assert_eq!(out, b"@@@!4A?");
    }

    #[test]
    fn a_flat_picture_needs_one_colour() {
        assert_eq!(median_cut(&[[5, 6, 7]; 50], 256), [[5, 6, 7]]);
        assert_eq!(median_cut(&[], 256), [[0, 0, 0]]);
    }
}
