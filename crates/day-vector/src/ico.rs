//! Windows `.ico` writer — the modern PNG-compressed form: an ICONDIR header, one 16-byte
//! entry per size, then the PNG blobs verbatim. ~60 lines is exactly why this is hand-rolled
//! rather than a dependency (minimal-dependencies policy).

/// Pack `(pixel_size, png_bytes)` entries into a multi-size `.ico`. Sizes above 256 are
/// skipped (the format caps at 256; 256 is encoded as width byte 0 per the spec).
pub fn pack_ico(entries: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let entries: Vec<&(u32, Vec<u8>)> = entries.iter().filter(|(s, _)| *s <= 256).collect();
    let count = entries.len() as u16;
    let mut out = Vec::new();
    // ICONDIR: reserved, type 1 (icon), count.
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    let mut offset = 6 + 16 * entries.len() as u32;
    for (size, png) in &entries {
        let edge = if *size >= 256 { 0u8 } else { *size as u8 };
        out.push(edge); // width (0 = 256)
        out.push(edge); // height
        out.push(0); // palette count
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // color planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in &entries {
        out.extend_from_slice(png);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ico_layout_round_trips() {
        let png16 = vec![0x89, b'P', b'N', b'G', 1, 2, 3];
        let png256 = vec![0x89, b'P', b'N', b'G', 9, 9];
        let ico = pack_ico(&[(16, png16.clone()), (256, png256.clone()), (512, vec![0])]);
        // 512 skipped → 2 entries.
        assert_eq!(u16::from_le_bytes([ico[4], ico[5]]), 2);
        // Entry 0: 16×16; entry 1: width byte 0 = 256.
        assert_eq!(ico[6], 16);
        assert_eq!(ico[6 + 16], 0);
        // First blob offset = 6 + 32, and the blob is the PNG verbatim.
        let off = u32::from_le_bytes(ico[6 + 12..6 + 16].try_into().unwrap()) as usize;
        assert_eq!(off, 38);
        assert_eq!(&ico[off..off + png16.len()], &png16[..]);
    }
}
