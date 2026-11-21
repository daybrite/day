// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! A small PNG reader for the `sample_pixel` step (docs/testing.md): the in-process capture
//! arrives as PNG bytes, and a test asks for the color of one point of it. It reads the
//! formats a window capture is written in, 8- and 16-bit gray, gray-alpha, RGB and RGBA,
//! non-interlaced, into RGBA8, with its own inflate (RFC 1951) rather than a dependency: this
//! engine is linked into every app, and the reader is a few hundred lines.

/// A decoded image: `rgba` holds `width * height` pixels, four bytes each, row by row.
pub struct Rgba {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl Rgba {
    /// The pixel at (`x`, `y`) as `[r, g, b, a]`, or `None` outside the image.
    #[must_use]
    pub fn pixel(&self, x: usize, y: usize) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = (y * self.width + x) * 4;
        Some([
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ])
    }
}

/// A PNG read as far as its header: the size is known, the pixels are not decoded yet.
///
/// `sample_pixel` wants one point of a window capture. Decoding the whole capture for it held
/// three full copies of a 2560×1600 window (the inflated stream, the unfiltered rows and the
/// RGBA expansion) and took about a second and a half per step in a debug build; a
/// [`pixel`](Png::pixel) read stops inflating at the pixel's row and keeps two rows.
pub struct Png {
    pub width: usize,
    pub height: usize,
    channels: usize,
    bytes_per_sample: usize,
    /// The zlib stream: every IDAT body, joined.
    data: Vec<u8>,
}

impl Png {
    /// Read `bytes`'s header and gather its image data, without inflating it.
    ///
    /// # Errors
    ///
    /// Returns an error when `bytes` is not a PNG, is truncated, or uses a color type, bit
    /// depth or interlacing this reader does not support.
    pub fn read(bytes: &[u8]) -> Result<Png, String> {
        const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        if bytes.len() < 8 || bytes[..8] != SIGNATURE {
            return Err("not a PNG".into());
        }
        let mut pos = 8;
        let mut header: Option<(usize, usize, u8, u8, u8)> = None;
        let mut data = Vec::new();
        while pos + 8 <= bytes.len() {
            let len =
                u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
                    as usize;
            let kind = &bytes[pos + 4..pos + 8];
            let body = bytes
                .get(pos + 8..pos + 8 + len)
                .ok_or("truncated PNG chunk")?;
            match kind {
                b"IHDR" => {
                    if body.len() < 13 {
                        return Err("short IHDR".into());
                    }
                    let be = |i: usize| {
                        u32::from_be_bytes([body[i], body[i + 1], body[i + 2], body[i + 3]])
                            as usize
                    };
                    header = Some((be(0), be(4), body[8], body[9], body[12]));
                }
                b"IDAT" => data.extend_from_slice(body),
                b"IEND" => break,
                _ => {}
            }
            // Length, type, body and CRC.
            pos += 12 + len;
        }
        let (width, height, depth, color, interlace) = header.ok_or("no IHDR")?;
        if interlace != 0 {
            return Err("interlaced PNG".into());
        }
        let channels = match color {
            0 => 1,
            2 => 3,
            4 => 2,
            6 => 4,
            _ => return Err(format!("PNG color type {color}")),
        };
        if depth != 8 && depth != 16 {
            return Err(format!("PNG bit depth {depth}"));
        }
        Ok(Png {
            width,
            height,
            channels,
            bytes_per_sample: usize::from(depth / 8),
            data,
        })
    }

    /// The pixel at (`x`, `y`) as `[r, g, b, a]`, or `None` outside the image. Only the rows
    /// down to `y` are inflated and unfiltered.
    ///
    /// # Errors
    ///
    /// Returns an error when the image data is truncated or corrupt.
    pub fn pixel(&self, x: usize, y: usize) -> Result<Option<[u8; 4]>, String> {
        if x >= self.width || y >= self.height {
            return Ok(None);
        }
        let bpp = self.bpp();
        let mut found = None;
        self.rows(y + 1, |row, line| {
            if row == y {
                found = Some(self.rgba_of(&line[x * bpp..(x + 1) * bpp]));
            }
        })?;
        Ok(found)
    }

    /// Bytes per pixel.
    fn bpp(&self) -> usize {
        self.channels * self.bytes_per_sample
    }

    /// One pixel's samples as RGBA8.
    fn rgba_of(&self, px: &[u8]) -> [u8; 4] {
        // The high byte of a 16-bit sample is its 8-bit value.
        let s = |i: usize| px[i * self.bytes_per_sample];
        match self.channels {
            1 => [s(0), s(0), s(0), 255],
            2 => [s(0), s(0), s(0), s(1)],
            3 => [s(0), s(1), s(2), 255],
            _ => [s(0), s(1), s(2), s(3)],
        }
    }

    /// Inflate and unfilter the first `rows` rows, handing each to `row` in order. Only the
    /// row being built and the one above it, which the filters read, are kept.
    fn rows(&self, rows: usize, mut row: impl FnMut(usize, &[u8])) -> Result<(), String> {
        let bpp = self.bpp();
        let stride = self.width * bpp;
        // Each row is a filter byte, then its samples.
        let need = rows * (stride + 1);
        // A zlib stream: a two-byte header, the deflate data, then a checksum this ignores.
        let raw = inflate_up_to(self.data.get(2..).ok_or("empty IDAT")?, need)?;
        if raw.len() < need {
            return Err("short image data".into());
        }
        let mut above = vec![0u8; stride];
        let mut line = vec![0u8; stride];
        for (y, filtered) in raw[..need].chunks_exact(stride + 1).enumerate() {
            let (filter, samples) = (filtered[0], &filtered[1..]);
            for x in 0..stride {
                let a = if x >= bpp { line[x - bpp] } else { 0 };
                let b = above[x];
                let c = if x >= bpp { above[x - bpp] } else { 0 };
                let v = samples[x];
                line[x] = match filter {
                    0 => v,
                    1 => v.wrapping_add(a),
                    2 => v.wrapping_add(b),
                    3 => v.wrapping_add(u16::midpoint(u16::from(a), u16::from(b)) as u8),
                    4 => v.wrapping_add(paeth(a, b, c)),
                    f => return Err(format!("PNG filter {f}")),
                };
            }
            row(y, &line);
            std::mem::swap(&mut above, &mut line);
        }
        Ok(())
    }
}

/// Decode `bytes` as a PNG.
///
/// # Errors
///
/// Returns an error when `bytes` is not a PNG, is truncated or corrupt, or uses a color type, bit
/// depth or interlacing this decoder does not support.
pub fn decode(bytes: &[u8]) -> Result<Rgba, String> {
    let png = Png::read(bytes)?;
    let bpp = png.bpp();
    let mut rgba = Vec::with_capacity(png.width * png.height * 4);
    png.rows(png.height, |_, line| {
        for px in line.chunks_exact(bpp) {
            rgba.extend_from_slice(&png.rgba_of(px));
        }
    })?;
    Ok(Rgba {
        width: png.width,
        height: png.height,
        rgba,
    })
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = (
        (p - i16::from(a)).abs(),
        (p - i16::from(b)).abs(),
        (p - i16::from(c)).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

// ---------------------------------------------------------------------------
// Inflate (RFC 1951), after zlib's `puff`: canonical Huffman codes decoded a bit at a time.
// ---------------------------------------------------------------------------

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u32,
    count: u32,
}

impl Bits<'_> {
    fn need(&mut self, n: u32) -> Result<u32, String> {
        let mut val = self.bit;
        while self.count < n {
            let byte = *self.data.get(self.pos).ok_or("deflate data ran out")?;
            self.pos += 1;
            val |= u32::from(byte) << self.count;
            self.count += 8;
        }
        self.bit = val >> n;
        self.count -= n;
        Ok(val & ((1u32 << n) - 1))
    }
}

/// A canonical Huffman code: how many codes of each length, and the symbols in code order.
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Huffman {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[usize::from(l)] += 1;
        }
        counts[0] = 0;
        let mut offs = [0u16; 16];
        for i in 1..16 {
            offs[i] = offs[i - 1] + counts[i - 1];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[usize::from(offs[usize::from(l)])] = sym as u16;
                offs[usize::from(l)] += 1;
            }
        }
        Huffman { counts, symbols }
    }

    fn decode(&self, bits: &mut Bits) -> Result<u16, String> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= bits.need(1)? as i32;
            let count = i32::from(self.counts[len]);
            if code - count < first {
                return self
                    .symbols
                    .get((index + (code - first)) as usize)
                    .copied()
                    .ok_or_else(|| "bad Huffman symbol".to_string());
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err("bad Huffman code".into())
    }
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Inflate a raw deflate stream.
///
/// # Errors
///
/// Returns an error when the stream is truncated or is not valid deflate data.
pub fn inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    inflate_up_to(data, usize::MAX)
}

/// The most [`inflate_up_to`] reserves ahead: a header's claim, not yet borne out by data.
const MAX_RESERVE: usize = 64 << 20;

/// Inflate a raw deflate stream until at least `limit` bytes are out (or the stream ends),
/// into a buffer reserved for `limit` up front rather than grown by doubling.
fn inflate_up_to(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    let mut bits = Bits {
        data,
        pos: 0,
        bit: 0,
        count: 0,
    };
    let mut out = Vec::with_capacity(limit.min(MAX_RESERVE));
    loop {
        if out.len() >= limit {
            return Ok(out);
        }
        let last = bits.need(1)?;
        match bits.need(2)? {
            0 => {
                // Stored: byte-aligned, a length and its complement, then the bytes as they are.
                bits.bit = 0;
                bits.count = 0;
                let p = bits.pos;
                let hdr = data.get(p..p + 4).ok_or("short stored block")?;
                let len = usize::from(u16::from_le_bytes([hdr[0], hdr[1]]));
                let body = data.get(p + 4..p + 4 + len).ok_or("short stored block")?;
                out.extend_from_slice(body);
                bits.pos = p + 4 + len;
            }
            1 => {
                let mut lengths = [0u8; 288];
                lengths[..144].fill(8);
                lengths[144..256].fill(9);
                lengths[256..280].fill(7);
                lengths[280..].fill(8);
                let lit = Huffman::new(&lengths);
                let dist = Huffman::new(&[5u8; 30]);
                codes(&mut bits, &mut out, &lit, &dist, limit)?;
            }
            2 => {
                const ORDER: [usize; 19] = [
                    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
                ];
                let nlen = bits.need(5)? as usize + 257;
                let ndist = bits.need(5)? as usize + 1;
                let ncode = bits.need(4)? as usize + 4;
                let mut code_lengths = [0u8; 19];
                for &i in ORDER.iter().take(ncode) {
                    code_lengths[i] = bits.need(3)? as u8;
                }
                let lencode = Huffman::new(&code_lengths);
                let mut lengths = vec![0u8; nlen + ndist];
                let mut i = 0;
                while i < nlen + ndist {
                    let sym = lencode.decode(&mut bits)?;
                    let (value, repeat) = match sym {
                        0..=15 => (sym as u8, 1),
                        16 => {
                            let prev = *lengths
                                .get(i.wrapping_sub(1))
                                .ok_or("repeat with no previous length")?;
                            (prev, 3 + bits.need(2)? as usize)
                        }
                        17 => (0, 3 + bits.need(3)? as usize),
                        _ => (0, 11 + bits.need(7)? as usize),
                    };
                    for _ in 0..repeat {
                        *lengths.get_mut(i).ok_or("too many code lengths")? = value;
                        i += 1;
                    }
                }
                let lit = Huffman::new(&lengths[..nlen]);
                let dist = Huffman::new(&lengths[nlen..]);
                codes(&mut bits, &mut out, &lit, &dist, limit)?;
            }
            _ => return Err("bad deflate block type".into()),
        }
        if last == 1 {
            return Ok(out);
        }
    }
}

/// Decode one block's codes into `out`, stopping early once `out` holds `limit` bytes.
fn codes(
    bits: &mut Bits,
    out: &mut Vec<u8>,
    lit: &Huffman,
    dist: &Huffman,
    limit: usize,
) -> Result<(), String> {
    loop {
        if out.len() >= limit {
            return Ok(());
        }
        let sym = lit.decode(bits)?;
        match sym {
            0..=255 => out.push(sym as u8),
            256 => return Ok(()),
            _ => {
                let i = usize::from(sym - 257);
                let len = usize::from(*LEN_BASE.get(i).ok_or("bad length code")?)
                    + bits.need(u32::from(LEN_EXTRA[i]))? as usize;
                let d = usize::from(dist.decode(bits)?);
                let back = usize::from(*DIST_BASE.get(d).ok_or("bad distance code")?)
                    + bits.need(u32::from(DIST_EXTRA[d]))? as usize;
                if back > out.len() {
                    return Err("distance before the start".into());
                }
                let from = out.len() - back;
                if back >= len {
                    out.extend_from_within(from..from + len);
                } else {
                    // An overlapping copy repeats the bytes it has just written.
                    for k in 0..len {
                        let b = out[from + k];
                        out.push(b);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Png, decode};

    /// Encode with the `png` crate (a dev-dependency only), decode with this reader.
    fn roundtrip(
        color: png::ColorType,
        depth: png::BitDepth,
        w: u32,
        h: u32,
        data: &[u8],
    ) -> super::Rgba {
        let mut buf = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut buf, w, h);
            enc.set_color(color);
            enc.set_depth(depth);
            enc.set_compression(png::Compression::Balanced);
            let mut writer = enc.write_header().expect("header");
            writer.write_image_data(data).expect("data");
        }
        decode(&buf).expect("decodes")
    }

    #[test]
    fn reads_rgba_and_rgb_with_every_filter_the_encoder_picks() {
        let (w, h) = (37u32, 23u32);
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&[(x * 7) as u8, (y * 11) as u8, ((x ^ y) * 5) as u8, 200]);
            }
        }
        let img = roundtrip(png::ColorType::Rgba, png::BitDepth::Eight, w, h, &data);
        assert_eq!((img.width, img.height), (37, 23));
        assert_eq!(img.rgba, data);

        let rgb: Vec<u8> = data.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        let img = roundtrip(png::ColorType::Rgb, png::BitDepth::Eight, w, h, &rgb);
        assert_eq!(img.pixel(3, 4), Some([21, 44, 35, 255]));
    }

    #[test]
    fn reads_gray_and_sixteen_bit_and_refuses_what_it_cannot() {
        let img = roundtrip(
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            2,
            1,
            &[9, 250],
        );
        assert_eq!(img.pixel(1, 0), Some([250, 250, 250, 255]));
        let img = roundtrip(
            png::ColorType::Rgb,
            png::BitDepth::Sixteen,
            1,
            1,
            &[1, 0, 2, 0, 3, 0],
        );
        assert_eq!(img.pixel(0, 0), Some([1, 2, 3, 255]));
        assert!(img.pixel(1, 0).is_none());
        assert!(decode(b"not a png").is_err());
    }

    #[test]
    fn one_pixel_reads_the_same_as_a_whole_decode() {
        let (w, h) = (41u32, 29u32);
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&[(x * 13) as u8, (y * 7) as u8, ((x * y) % 251) as u8, 99]);
            }
        }
        let mut buf = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut buf, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Balanced);
            let mut writer = enc.write_header().expect("header");
            writer.write_image_data(&data).expect("data");
        }
        let whole = decode(&buf).expect("decodes");
        let png = Png::read(&buf).expect("reads");
        assert_eq!((png.width, png.height), (41, 29));
        for (x, y) in [(0, 0), (40, 0), (17, 11), (0, 28), (40, 28)] {
            assert_eq!(png.pixel(x, y).expect("inflates"), whole.pixel(x, y));
        }
        assert_eq!(png.pixel(41, 0).expect("in range check"), None);
        assert_eq!(png.pixel(0, 29).expect("in range check"), None);
    }

    #[test]
    fn a_large_flat_image_inflates_through_long_back_references() {
        let (w, h) = (300u32, 200u32);
        let data = vec![128u8; (w * h * 4) as usize];
        let img = roundtrip(png::ColorType::Rgba, png::BitDepth::Eight, w, h, &data);
        assert_eq!(img.pixel(299, 199), Some([128, 128, 128, 128]));
    }
}
