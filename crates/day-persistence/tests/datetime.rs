// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The `datetime` feature's codecs: `DayDate` and `DayTime` as columns.

#![cfg(feature = "datetime")]

use day_persistence::datetime::{EpochMillis, EpochSeconds, Iso8601};
use day_persistence::{ColumnValue, Value, ValueCodec};
use day_piece_datetime::{DayDate, DayTime};

/// Round-trip identity for every codec, and order agreement for the ones that sort.
#[test]
fn sqlite_codecs_round_trip_and_keep_order() {
    let dates = [
        DayDate::default(),
        DayDate::new(1969, 12, 31).unwrap(),
        DayDate::new(2026, 8, 20).unwrap(),
        DayDate::new(2000, 2, 29).unwrap(),
        DayDate::new(1, 1, 1).unwrap(),
    ];
    for d in dates {
        assert_eq!(DayDate::from_sqlite_value(d.to_sqlite_value()).unwrap(), d);
        assert_eq!(
            <Iso8601 as ValueCodec<DayDate>>::from_sqlite_value(
                <Iso8601 as ValueCodec<DayDate>>::to_sqlite_value(&d)
            )
            .unwrap(),
            d
        );
        assert_eq!(
            <EpochSeconds as ValueCodec<DayDate>>::from_sqlite_value(
                <EpochSeconds as ValueCodec<DayDate>>::to_sqlite_value(&d)
            )
            .unwrap(),
            d
        );
        assert_eq!(
            <EpochMillis as ValueCodec<DayDate>>::from_sqlite_value(<EpochMillis as ValueCodec<
                DayDate,
            >>::to_sqlite_value(
                &d
            ))
            .unwrap(),
            d
        );
    }
    // ISO-8601's design goal: lexicographic == chronological (positive years).
    let a = <Iso8601 as ValueCodec<DayDate>>::to_sqlite_value(&dates[1]);
    let b = <Iso8601 as ValueCodec<DayDate>>::to_sqlite_value(&dates[2]);
    if let (Value::Text(a), Value::Text(b)) = (a, b) {
        assert!(a < b);
    } else {
        panic!("Iso8601 stores TEXT");
    }

    let times = [
        DayTime::default(),
        DayTime::new(9, 30, 15).unwrap(),
        DayTime::new(23, 59, 59).unwrap(),
    ];
    for t in times {
        assert_eq!(DayTime::from_sqlite_value(t.to_sqlite_value()).unwrap(), t);
        assert_eq!(
            <Iso8601 as ValueCodec<DayTime>>::from_sqlite_value(
                <Iso8601 as ValueCodec<DayTime>>::to_sqlite_value(&t)
            )
            .unwrap(),
            t
        );
    }

    // A malformed stored string is a Decode error, not a panic or a silent default.
    assert!(
        <Iso8601 as ValueCodec<DayDate>>::from_sqlite_value(Value::Text("08/20/2026".into()))
            .is_err()
    );
}
