// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-part-timezone — a HEADLESS cross-platform API for the host's **wall clock** and **time-zone
//! facts**. No UI; any Rust code can depend on this crate to ask "what time is it", "what zone is
//! this device in", and "what is zone X's UTC offset at instant T" (DST-correct, from the bundled
//! IANA database).
//!
//! ```no_run
//! let at = day_part_timezone::now();
//! if let Some(off) = day_part_timezone::offset_seconds("Asia/Tokyo", at) {
//!     println!("Tokyo is UTC{:+}s right now", off);
//! }
//! ```
//!
//! Day's core is deliberately zoneless — day-l10n formats epochs as UTC civil time and
//! day-piece-datetime edits zoneless values — so zone-aware apps (world clocks, alarms, calendars)
//! get their zone arithmetic here instead. The API stays in instants and offsets: apply an offset
//! to an epoch and hand the shifted value to Fluent's `DATETIME` for locale-correct rendering
//! (docs/timezone.md shows the pattern).
//!
//! Platform selection is `#[cfg]` on `wasm32` versus everything else. Offsets come from a bundled
//! IANA database (jiff, `tzdb-bundle-always`), so they answer identically on all targets with no
//! OS zoneinfo needed. The host-fact functions differ per target:
//!
//! - [`now`] — `SystemTime::now()` everywhere except `wasm32`, where std has no clock and the
//!   day-dom shim answers with the page's `Date.now()`.
//! - [`local_zone`] — jiff's system detection (`/etc/localtime`, the Windows registry, Android's
//!   `persist.sys.timezone`) with a documented fall-back to `"UTC"` when the OS won't say; on
//!   `wasm32` the shim's `tz` env key carries `Intl.DateTimeFormat().resolvedOptions().timeZone`
//!   (`None` if the browser gives nothing usable).
//!
//! Everything is best-effort and non-panicking: unknown zones answer `None`, never an error.

use std::time::{SystemTime, UNIX_EPOCH};

/// The current wall-clock instant. Identical to `SystemTime::now()` on every target except
/// `web-dom` (`wasm32`), where `SystemTime::now()` aborts and this asks the host page's
/// `Date.now()` instead. Use this — not `SystemTime::now()` — in code that ships to web.
pub fn now() -> SystemTime {
    #[cfg(not(target_arch = "wasm32"))]
    {
        SystemTime::now()
    }
    #[cfg(target_arch = "wasm32")]
    {
        UNIX_EPOCH + std::time::Duration::from_millis(day_dom::now_epoch_ms())
    }
}

/// [`now`] as milliseconds since the Unix epoch — the convenient shape for stored anchors
/// (stopwatch starts, timer deadlines) and for Fluent `DATETIME` arguments. Answers `0` in the
/// pre-1970 clock-is-broken case rather than panicking.
pub fn now_epoch_ms() -> u64 {
    epoch_ms(now())
}

/// A `SystemTime` as milliseconds since the Unix epoch (`0` for pre-epoch instants).
pub fn epoch_ms(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The device's current IANA time-zone id (`"America/New_York"`), or `None` when the platform
/// won't say. When the OS *has* a zone but no IANA name for it, this answers `"UTC"` (jiff's
/// documented fall-back) — callers that only need the local offset should prefer
/// [`local_offset_seconds`], which stays correct even then.
pub fn local_zone() -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        jiff::tz::TimeZone::system().iana_name().map(str::to_owned)
    }
    #[cfg(target_arch = "wasm32")]
    {
        day_dom::host_env("tz").filter(|z| is_zone(z))
    }
}

/// Whether `zone` names a zone in the bundled IANA database (case-insensitive per IANA rules,
/// so `"asia/tokyo"` answers `true`).
pub fn is_zone(zone: &str) -> bool {
    jiff::tz::TimeZone::get(zone).is_ok()
}

/// `zone`'s UTC offset in seconds at the instant `at`, DST-correct from the bundled IANA database
/// (east positive: Tokyo answers `32400`, New York `-18000` in winter and `-14400` in summer).
/// `None` when `zone` isn't in the database or `at` is outside jiff's representable range.
pub fn offset_seconds(zone: &str, at: SystemTime) -> Option<i32> {
    let tz = jiff::tz::TimeZone::get(zone).ok()?;
    Some(tz.to_offset(timestamp(at)?).seconds())
}

/// The device zone's UTC offset in seconds at the instant `at`. Unlike
/// `offset_seconds(&local_zone()?, at)` this stays correct on hosts whose zone has no IANA name
/// (it asks the system zone directly). `None` only on `web-dom` when the browser reports no zone,
/// or for out-of-range instants.
pub fn local_offset_seconds(at: SystemTime) -> Option<i32> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Some(
            jiff::tz::TimeZone::system()
                .to_offset(timestamp(at)?)
                .seconds(),
        )
    }
    #[cfg(target_arch = "wasm32")]
    {
        offset_seconds(&local_zone()?, at)
    }
}

/// A `SystemTime` as a jiff `Timestamp` (`None` outside jiff's ±9999-year range).
fn timestamp(at: SystemTime) -> Option<jiff::Timestamp> {
    jiff::Timestamp::try_from(at).ok()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::time::Duration;

    /// An instant from UTC civil time, built with jiff's civil API. Offset EXPECTATIONS in these
    /// tests are independent constants (IANA rules known from the tzdata release notes), so the
    /// library isn't testing itself.
    fn utc(y: i16, mo: i8, d: i8, h: i8, mi: i8) -> SystemTime {
        let ts = jiff::civil::date(y, mo, d)
            .at(h, mi, 0, 0)
            .to_zoned(jiff::tz::TimeZone::UTC)
            .expect("in-range civil fixture")
            .timestamp();
        UNIX_EPOCH + Duration::from_secs(ts.as_second() as u64)
    }

    #[test]
    fn dst_spring_forward_new_york() {
        // US DST 2026 begins 2026-03-08 at 02:00 EST → 03:00 EDT, i.e. 07:00 UTC.
        let before = utc(2026, 3, 8, 6, 59);
        let after = utc(2026, 3, 8, 7, 1);
        assert_eq!(offset_seconds("America/New_York", before), Some(-5 * 3600));
        assert_eq!(offset_seconds("America/New_York", after), Some(-4 * 3600));
    }

    #[test]
    fn southern_hemisphere_reversed() {
        // Sydney observes DST in the southern summer: AEDT (+11) in January, AEST (+10) in July.
        assert_eq!(
            offset_seconds("Australia/Sydney", utc(2026, 1, 15, 0, 0)),
            Some(11 * 3600)
        );
        assert_eq!(
            offset_seconds("Australia/Sydney", utc(2026, 7, 15, 0, 0)),
            Some(10 * 3600)
        );
    }

    #[test]
    fn fixed_and_fractional_offsets() {
        assert_eq!(offset_seconds("UTC", utc(2026, 2, 13, 17, 41)), Some(0));
        // Kathmandu's +05:45 never changes; Tehran dropped DST in 2022, so +03:30 year-round.
        assert_eq!(
            offset_seconds("Asia/Kathmandu", utc(2026, 6, 1, 0, 0)),
            Some(5 * 3600 + 45 * 60)
        );
        assert_eq!(
            offset_seconds("Asia/Tehran", utc(2026, 7, 1, 0, 0)),
            Some(3 * 3600 + 30 * 60)
        );
    }

    #[test]
    fn unknown_zone_answers_none() {
        assert_eq!(offset_seconds("Mars/Olympus_Mons", now()), None);
        assert!(!is_zone("Mars/Olympus_Mons"));
        assert!(!is_zone(""));
    }

    #[test]
    fn zone_lookup_is_case_insensitive() {
        assert!(is_zone("asia/tokyo"));
        assert_eq!(
            offset_seconds("asia/tokyo", utc(2026, 2, 13, 17, 41)),
            Some(9 * 3600)
        );
    }

    #[test]
    fn local_zone_and_offset_agree() {
        // Whatever this host's zone is, the two local accessors must tell one story.
        let at = utc(2026, 2, 13, 17, 41);
        let direct = local_offset_seconds(at).expect("native hosts always answer");
        if let Some(zone) = local_zone() {
            assert_eq!(offset_seconds(&zone, at), Some(direct));
        }
        // Offsets are always within ±18h (the IANA envelope).
        assert!(direct.abs() <= 18 * 3600);
    }

    #[test]
    fn clock_runs() {
        assert!(now_epoch_ms() > 1_700_000_000_000); // after 2023 — the clock is real, not zero
        assert_eq!(epoch_ms(UNIX_EPOCH), 0);
        assert_eq!(epoch_ms(UNIX_EPOCH - Duration::from_secs(1)), 0); // pre-epoch clamps, no panic
    }

    #[test]
    fn common_world_clock_zones_resolve() {
        // The high-traffic ids a world clock leans on (the app's full dataset asserts all 248).
        for z in [
            "America/New_York",
            "America/Los_Angeles",
            "America/Sao_Paulo",
            "Europe/London",
            "Europe/Paris",
            "Europe/Moscow",
            "Africa/Cairo",
            "Africa/Lagos",
            "Asia/Dubai",
            "Asia/Kolkata",
            "Asia/Shanghai",
            "Asia/Tokyo",
            "Australia/Sydney",
            "Pacific/Auckland",
            "America/St_Johns",
            "Asia/Yangon",
        ] {
            assert!(is_zone(z), "{z} missing from bundled tzdb");
        }
    }
}
