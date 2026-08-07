//! macOS `.icns` writer — the modern PNG-chunk form: an `icns` header plus one four-cc chunk
//! per size, each carrying a PNG verbatim. Only the documented PNG-only OSTypes are emitted
//! (`ic07`–`ic14`), so every reader from Finder to `iconutil` accepts the file; sizes without
//! a PNG-only slot (bare 16 px) are skipped — the 2x members cover those densities.

/// The PNG-only icns members: pixel size → OSType.
fn ostype(px: u32) -> Option<&'static [u8; 4]> {
    match px {
        32 => Some(b"ic11"),   // 16 pt @2x
        64 => Some(b"ic12"),   // 32 pt @2x
        128 => Some(b"ic07"),  // 128 pt
        256 => Some(b"ic08"),  // 256 pt
        512 => Some(b"ic09"),  // 512 pt
        1024 => Some(b"ic10"), // 512 pt @2x
        _ => None,
    }
}

/// Pack `(pixel_size, png_bytes)` entries into an `.icns`. Entries without a PNG-only slot are
/// skipped; at least one must remain.
pub fn pack_icns(entries: &[(u32, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let chunks: Vec<(&[u8; 4], &Vec<u8>)> = entries
        .iter()
        .filter_map(|(px, png)| ostype(*px).map(|t| (t, png)))
        .collect();
    if chunks.is_empty() {
        return Err("no icns-representable sizes (need one of 32/64/128/256/512/1024)".into());
    }
    let total: u32 = 8 + chunks.iter().map(|(_, p)| 8 + p.len() as u32).sum::<u32>();
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&total.to_be_bytes());
    for (t, png) in chunks {
        out.extend_from_slice(t.as_slice());
        out.extend_from_slice(&(8 + png.len() as u32).to_be_bytes());
        out.extend_from_slice(png);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icns_layout_round_trips() {
        let png = vec![0x89, b'P', b'N', b'G', 7, 7, 7];
        let icns = pack_icns(&[(1024, png.clone()), (17, vec![1])]).unwrap();
        assert_eq!(&icns[0..4], b"icns");
        let total = u32::from_be_bytes(icns[4..8].try_into().unwrap()) as usize;
        assert_eq!(total, icns.len());
        assert_eq!(&icns[8..12], b"ic10");
        let clen = u32::from_be_bytes(icns[12..16].try_into().unwrap()) as usize;
        assert_eq!(clen, 8 + png.len());
        assert_eq!(&icns[16..16 + png.len()], &png[..]);
    }

    #[test]
    fn unrepresentable_only_is_an_error() {
        assert!(pack_icns(&[(16, vec![0])]).is_err());
    }
}
