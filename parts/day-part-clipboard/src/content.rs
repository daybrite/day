// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! One clipboard item with alternate MIME representations, including arbitrary bytes.
use std::{future::Future, pin::Pin, sync::Arc};

/// One MIME representation. Bytes are encoded content, never a native image handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Representation {
    pub mime: String,
    pub bytes: Arc<Vec<u8>>,
}
impl Representation {
    pub fn new(mime: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            mime: mime.into(),
            bytes: Arc::new(bytes.into()),
        }
    }
}
/// Alternative representations of ONE logical item, ordered by preference.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Content(pub Vec<Representation>);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Unsupported,
    Unavailable,
    InvalidData,
    TooLarge,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "clipboard: {self:?}")
    }
}
impl std::error::Error for Error {}
pub type ClipboardFuture<T> = Pin<Box<dyn Future<Output = Result<T, Error>>>>;
/// Resource limit applied before allocating clipboard content from another application.
pub const MAX_BYTES: usize = 64 * 1024 * 1024;

impl Content {
    pub fn validate(&self) -> Result<(), Error> {
        if self.0.is_empty() || self.0.len() > 32 {
            return Err(Error::InvalidData);
        }
        let mut total = 0usize;
        let mut types = std::collections::HashSet::new();
        for r in &self.0 {
            if !valid_mime(&r.mime) || !types.insert(&r.mime) {
                return Err(Error::InvalidData);
            }
            total = total.checked_add(r.bytes.len()).ok_or(Error::TooLarge)?;
            if total > MAX_BYTES {
                return Err(Error::TooLarge);
            }
        }
        Ok(())
    }
}
pub(crate) fn valid_mime(m: &str) -> bool {
    m.len() <= 255
        && m.split_once('/')
            .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty() && !b.contains('/'))
        && m.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"/!#$&^_.+-".contains(&c))
}
/// Read the first available type in caller preference order. Empty and denied are distinct.
/// Start this call from a user action; browsers capture the live paste event immediately.
pub fn read(preferred: &[&str]) -> ClipboardFuture<Option<Representation>> {
    if preferred.is_empty() || preferred.iter().any(|m| !valid_mime(m)) {
        return Box::pin(std::future::ready(Err(Error::InvalidData)));
    }
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    {
        super::web_content::read(preferred)
    }
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    {
        Box::pin(std::future::ready(super::imp::read_content(preferred)))
    }
}
/// Replace the clipboard with alternate representations. Returns the types actually written;
/// platforms with restricted formats may accept a subset. Cut only after this succeeds.
/// No previous-copy fallback is used: an empty/denied clipboard never resurrects stale data.
pub fn write(content: Content) -> ClipboardFuture<Vec<String>> {
    if let Err(e) = content.validate() {
        return Box::pin(std::future::ready(Err(e)));
    }
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    {
        super::web_content::write(content)
    }
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    {
        Box::pin(std::future::ready(super::imp::write_content(&content)))
    }
}

// Internal bridge wire format: count, then (MIME length, byte length, MIME, bytes), LE u32.
// It is transport only, never a replacement for standard system clipboard representations.
#[allow(dead_code)]
pub(crate) fn pack(content: &Content) -> Vec<u8> {
    let mut out = (content.0.len() as u32).to_le_bytes().to_vec();
    for r in &content.0 {
        out.extend_from_slice(&(r.mime.len() as u32).to_le_bytes());
        out.extend_from_slice(&(r.bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(r.mime.as_bytes());
        out.extend_from_slice(&r.bytes);
    }
    out
}
#[allow(dead_code)]
pub(crate) fn unpack(mut data: &[u8]) -> Result<Content, Error> {
    fn take<'a>(data: &mut &'a [u8], n: usize) -> Result<&'a [u8], Error> {
        if n > data.len() {
            return Err(Error::InvalidData);
        }
        let (head, tail) = data.split_at(n);
        *data = tail;
        Ok(head)
    }
    fn number(data: &mut &[u8]) -> Result<usize, Error> {
        Ok(u32::from_le_bytes(take(data, 4)?.try_into().unwrap()) as usize)
    }
    if data.len() > MAX_BYTES + 32 * 263 + 4 {
        return Err(Error::TooLarge);
    }
    let count = number(&mut data)?;
    if count > 32 {
        return Err(Error::InvalidData);
    }
    let mut out = Vec::new();
    for _ in 0..count {
        let m = number(&mut data)?;
        let n = number(&mut data)?;
        let mime = std::str::from_utf8(take(&mut data, m)?)
            .map_err(|_| Error::InvalidData)?
            .to_string();
        let bytes = take(&mut data, n)?.to_vec();
        out.push(Representation::new(mime, bytes));
    }
    if !data.is_empty() {
        return Err(Error::InvalidData);
    }
    let content = Content(out);
    if !content.0.is_empty() {
        content.validate()?;
    }
    Ok(content)
}

/// Convert Windows' packed DIB clipboard representation into an encoded BMP file.
#[cfg(any(windows, test))]
pub(crate) fn dib_to_bmp(dib: &[u8]) -> Result<Vec<u8>, Error> {
    if dib.len() < 12 || dib.len() > MAX_BYTES - 14 {
        return Err(Error::InvalidData);
    }
    let u32_at = |at: usize| -> Result<u32, Error> {
        Ok(u32::from_le_bytes(
            dib.get(at..at + 4)
                .ok_or(Error::InvalidData)?
                .try_into()
                .unwrap(),
        ))
    };
    let header = u32_at(0)? as usize;
    let (palette, masks) = if header == 12 {
        let bits = u16::from_le_bytes([dib[10], dib[11]]);
        (if bits <= 8 { (1usize << bits) * 3 } else { 0 }, 0)
    } else if header >= 40 && header <= dib.len() {
        let bits = u16::from_le_bytes([dib[14], dib[15]]);
        let colors = u32_at(32)? as usize;
        let entries = if colors != 0 {
            colors
        } else if bits <= 8 {
            1usize << bits
        } else {
            0
        };
        let masks = if header == 40 {
            match u32_at(16)? {
                3 => 12,
                6 => 16,
                _ => 0,
            }
        } else {
            0
        };
        (entries.checked_mul(4).ok_or(Error::InvalidData)?, masks)
    } else {
        return Err(Error::InvalidData);
    };
    let offset = header
        .checked_add(palette)
        .and_then(|n| n.checked_add(masks))
        .ok_or(Error::InvalidData)?;
    if offset > dib.len() {
        return Err(Error::InvalidData);
    }
    let mut bmp = Vec::with_capacity(dib.len() + 14);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&((dib.len() + 14) as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&((offset + 14) as u32).to_le_bytes());
    bmp.extend_from_slice(dib);
    Ok(bmp)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dib_clipboard_pixels_get_a_bmp_file_header() {
        let mut dib = vec![0u8; 44];
        dib[..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1u32.to_le_bytes());
        dib[8..12].copy_from_slice(&1u32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&24u16.to_le_bytes());
        dib[40..44].copy_from_slice(&[1, 2, 3, 0]);
        let bmp = dib_to_bmp(&dib).unwrap();
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(&bmp[10..14], &54u32.to_le_bytes());
        assert_eq!(&bmp[54..], &[1, 2, 3, 0]);
        assert!(dib_to_bmp(&dib[..20]).is_err());
        dib[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(dib_to_bmp(&dib).is_err());
    }
    #[test]
    fn binary_content_is_lossless_and_validated() {
        let c = Content(vec![Representation::new(
            "application/x-day-test",
            vec![0, 255, 0, 128],
        )]);
        assert_eq!(c.validate(), Ok(()));
        assert_eq!(&**c.0[0].bytes, &[0, 255, 0, 128]);
        assert_eq!(
            Content(vec![c.0[0].clone(), c.0[0].clone()]).validate(),
            Err(Error::InvalidData)
        );
        assert!(!valid_mime("image/png\0"));
        assert!(!valid_mime("image/png/extra"));
        assert!(!valid_mime("text/plain; charset=utf-8"));
        let shared = Arc::new(vec![0; MAX_BYTES / 32 + 1]);
        let oversized = Content(
            (0..32)
                .map(|i| Representation {
                    mime: format!("application/x-limit-{i}"),
                    bytes: shared.clone(),
                })
                .collect(),
        );
        assert_eq!(oversized.validate(), Err(Error::TooLarge));
        let packet = pack(&c);
        assert_eq!(unpack(&packet), Ok(c));
        for end in 0..packet.len() {
            assert!(unpack(&packet[..end]).is_err());
        }
        assert!(unpack(&[255, 255, 255, 255]).is_err());
    }
}
