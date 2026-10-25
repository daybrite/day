// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use serde::{Deserialize, Deserializer, Serialize};

/// A memory threshold in bytes. Accepts an integer byte count or a size string;
/// serialization uses bytes, so the wire is independent of the author's unit spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct MemorySize(pub u64);

impl<'de> Deserialize<'de> for MemorySize {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Input {
            Bytes(u64),
            Size(String),
        }
        match Input::deserialize(deserializer)? {
            Input::Bytes(bytes) => Ok(Self(bytes)),
            Input::Size(size) => parse(&size).map(Self).ok_or_else(|| {
                serde::de::Error::custom(
                    "memory size must be a non-negative whole byte count within u64, e.g. 500MB, 1G, or 512MiB",
                )
            }),
        }
    }
}

fn parse(size: &str) -> Option<u64> {
    let size = size.trim();
    let end = size
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(size.len());
    let (number, unit) = size.split_at(end);
    let multiplier: u128 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" => 1_000,
        "m" | "mb" => 1_000_000,
        "g" | "gb" => 1_000_000_000,
        "t" | "tb" => 1_000_000_000_000,
        "kib" => 1 << 10,
        "mib" => 1 << 20,
        "gib" => 1 << 30,
        "tib" => 1 << 40,
        _ => return None,
    };
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty()
        || !fraction.bytes().all(|c| c.is_ascii_digit())
        || (number.contains('.') && fraction.is_empty())
    {
        return None;
    }
    let scale = 10_u128.checked_pow(fraction.len().try_into().ok()?)?;
    let numerator = whole
        .parse::<u128>()
        .ok()?
        .checked_mul(scale)?
        .checked_add(if fraction.is_empty() {
            0
        } else {
            fraction.parse().ok()?
        })?;
    let bytes = numerator.checked_mul(multiplier)?;
    if bytes % scale != 0 {
        return None;
    }
    u64::try_from(bytes / scale).ok()
}
