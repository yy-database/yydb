//! Boundary and exactness tests for temporal and fixed-point scalars.

use std::cmp::Ordering;

use yydb_types::scalar::{self, Date, Decimal, ScalarError, Time, Timestamp};

#[test]
fn date_range_and_leap_year_are_exact() {
    assert_eq!(Date::MIN.to_iso(), "0001-01-01");
    assert_eq!(Date::MAX.to_iso(), "9999-12-31");
    assert_eq!(Date::MIN.days(), -719_162);
    assert_eq!(Date::MAX.days(), 2_932_896);
    assert_eq!(Date::parse_iso("1970-01-01").unwrap().days(), 0);
    assert_eq!(Date::parse_iso("2024-02-29").unwrap().to_ymd(), (2024, 2, 29));

    let leap_day = Date::parse_iso("2024-02-29").unwrap();
    assert_eq!(Date::from_days(leap_day.days()).unwrap(), leap_day);
    assert_eq!(Date::from_ymd(2023, 2, 29).unwrap_err().to_string(), "date out of range: day 29 is outside 1..=28 for 2023-02");
    assert!(matches!(Date::from_ymd(0, 1, 1), Err(ScalarError::OutOfRange { .. })));
    assert!(matches!(Date::from_ymd(10_000, 1, 1), Err(ScalarError::OutOfRange { .. })));
    assert!(matches!(Date::parse_iso("2024-13-01"), Err(ScalarError::OutOfRange { .. })));
    assert!(matches!(Date::parse_iso("2024-1-01"), Err(ScalarError::Malformed { .. })));
    assert!(matches!(Date::parse_iso("2024-01-01 "), Err(ScalarError::Malformed { .. })));
    assert!(matches!(Date::parse_iso("２０２４-01-01"), Err(ScalarError::Malformed { .. })));
    assert!(matches!(Date::from_days(Date::MAX.days() + 1), Err(ScalarError::OutOfRange { .. })));
}

#[test]
fn date_encoding_is_fixed_width_and_order_preserving() {
    let dates = [Date::MIN, Date::parse_iso("1969-12-31").unwrap(), Date::parse_iso("1970-01-01").unwrap(), Date::MAX];
    for pair in dates.windows(2) {
        assert!(pair[0].encode() < pair[1].encode(), "{} must sort before {}", pair[0], pair[1]);
    }
    for date in dates {
        assert_eq!(Date::decode(&date.encode()).unwrap(), date);
    }
    assert!(matches!(Date::decode(&[0, 0, 0]), Err(ScalarError::InvalidLength { expected: 4, got: 3, .. })));
    assert!(matches!(Date::decode(&[0xff, 0xff, 0xff, 0xff]), Err(ScalarError::OutOfRange { .. })));
}

#[test]
fn time_supports_nanoseconds_and_rejects_finer_input() {
    assert_eq!(Time::MIN.to_iso(), "00:00:00");
    assert_eq!(Time::MAX.to_iso(), "23:59:59.999999999");
    assert_eq!(Time::parse_iso("12:34:56.123456789").unwrap().nanos(), 45_296_123_456_789);
    assert_eq!(Time::parse_iso("12:34:56").unwrap().to_iso(), "12:34:56");
    assert!(matches!(Time::parse_iso("12:34:56.1234567890"), Err(ScalarError::Malformed { .. })));
    assert!(matches!(Time::parse_iso("24:00:00"), Err(ScalarError::OutOfRange { .. })));
    assert!(matches!(Time::parse_iso("12:60:00"), Err(ScalarError::OutOfRange { .. })));
    assert!(matches!(Time::parse_iso("12:34"), Err(ScalarError::Malformed { .. })));
    assert_eq!(Time::MIN.encode(), [0x80, 0, 0, 0, 0, 0, 0, 0]);
    for pair in [Time::MIN, Time::from_hms_nano(0, 0, 0, 1).unwrap(), Time::MAX].windows(2) {
        assert!(pair[0].encode() < pair[1].encode());
    }
    assert_eq!(Time::decode(&Time::MAX.encode()).unwrap(), Time::MAX);
    assert!(matches!(Time::decode(&[0; 7]), Err(ScalarError::InvalidLength { expected: 8, got: 7, .. })));
}

#[test]
fn timestamp_is_utc_with_microsecond_resolution() {
    let instant = Timestamp::parse_iso("2024-02-29T12:34:56.123456Z").unwrap();
    assert_eq!(instant.to_iso(), "2024-02-29T12:34:56.123456Z");
    assert_eq!(instant.to_parts(), (Date::parse_iso("2024-02-29").unwrap(), Time::parse_iso("12:34:56.123456").unwrap()));
    assert_eq!(Timestamp::parse_iso("2024-02-29T12:34:56.123456+00:00").unwrap(), instant);
    assert_eq!(Timestamp::MIN.to_iso(), "0001-01-01T00:00:00Z");
    assert_eq!(Timestamp::MAX.to_iso(), "9999-12-31T23:59:59.999999Z");
    assert!(matches!(Timestamp::parse_iso("2024-02-29T12:34:56+08:00"), Err(ScalarError::UnsupportedUtcOffset { .. })));
    assert!(matches!(Timestamp::parse_iso("2024-02-29T12:34:56.123456789Z"), Err(ScalarError::PrecisionLoss { .. })));
    assert!(matches!(Timestamp::from_unix_micros(Timestamp::MAX.unix_micros() + 1), Err(ScalarError::OutOfRange { .. })));
    assert!(matches!(Timestamp::from_unix_micros(Timestamp::MIN.unix_micros() - 1), Err(ScalarError::OutOfRange { .. })));
    for pair in [Timestamp::MIN, instant, Timestamp::MAX].windows(2) {
        assert!(pair[0].encode() < pair[1].encode());
    }
    assert_eq!(Timestamp::decode(&instant.encode()).unwrap(), instant);
}

#[test]
fn decimal_parses_exactly_and_rejects_overflow() {
    let value = Decimal::parse("-1234.50", 6, 2).unwrap();
    assert_eq!(value.unscaled(), -123_450);
    assert_eq!(value.precision(), 6);
    assert_eq!(value.scale(), 2);
    assert_eq!(value.to_plain_string(), "-1234.50");

    assert_eq!(Decimal::parse("-0.00", 5, 2).unwrap().unscaled(), 0);
    assert_eq!(Decimal::parse("-0.00", 5, 2).unwrap().to_plain_string(), "0.00");
    assert_eq!(Decimal::parse("0", 1, 0).unwrap().to_plain_string(), "0");
    assert_eq!(Decimal::parse(".5", 2, 1).unwrap().to_plain_string(), "0.5");

    assert!(matches!(Decimal::parse("1234.56", 5, 2), Err(ScalarError::PrecisionOverflow { .. })));
    assert!(matches!(Decimal::parse("1.234", 4, 2), Err(ScalarError::ScaleOverflow { .. })));
    assert!(matches!(Decimal::parse("1e3", 4, 1), Err(ScalarError::Malformed { .. })));
    assert!(matches!(Decimal::parse("1.", 4, 1), Err(ScalarError::Malformed { .. })));
    assert!(matches!(Decimal::parse("", 4, 1), Err(ScalarError::Malformed { .. })));
    assert!(matches!(Decimal::new(0, 0, 0), Err(ScalarError::InvalidPrecision { .. })));
    assert!(matches!(Decimal::new(0, 39, 0), Err(ScalarError::InvalidPrecision { .. })));
    assert!(matches!(Decimal::new(0, 2, 3), Err(ScalarError::InvalidScale { .. })));
    assert!(Decimal::parse("99999999999999999999999999999999999999", 38, 0).is_ok());
    assert!(matches!(
        Decimal::parse("100000000000000000000000000000000000000", 38, 0),
        Err(ScalarError::PrecisionOverflow { .. })
    ));
}

#[test]
fn decimal_compares_exactly_across_scales_and_signs() {
    let half = Decimal::parse("0.5", 4, 1).unwrap();
    let half_scaled = Decimal::parse("0.500", 4, 3).unwrap();
    let tenth = Decimal::parse("0.1", 4, 1).unwrap();
    let negative = Decimal::parse("-0.5", 4, 1).unwrap();
    let zero = Decimal::parse("0.00", 4, 2).unwrap();
    let zero_scaled = Decimal::parse("0.000", 4, 3).unwrap();

    assert_eq!(half.compare(half_scaled), Ordering::Equal);
    assert_eq!(half.compare(tenth), Ordering::Greater);
    assert_eq!(tenth.compare(half), Ordering::Less);
    assert_eq!(negative.compare(half), Ordering::Less);
    assert_eq!(negative.compare(Decimal::parse("-0.6", 4, 1).unwrap()), Ordering::Greater);
    assert_eq!(zero.compare(zero_scaled), Ordering::Equal);
    assert_eq!(negative.compare(zero), Ordering::Less);
    assert_eq!(zero.compare(half), Ordering::Less);
    assert_eq!(Decimal::parse("1.0", 4, 1).unwrap().compare(Decimal::parse("0.999", 4, 3).unwrap()), Ordering::Greater);
    assert_eq!(Decimal::parse("-10.0", 4, 1).unwrap().compare(Decimal::parse("-2.00", 4, 2).unwrap()), Ordering::Less);
}

#[test]
fn decimal_encoding_is_fixed_width_order_preserving_and_shape_checked() {
    let values = [
        Decimal::parse("-100.00", 6, 2).unwrap(),
        Decimal::parse("-1.00", 6, 2).unwrap(),
        Decimal::parse("-0.01", 6, 2).unwrap(),
        Decimal::parse("0.00", 6, 2).unwrap(),
        Decimal::parse("0.01", 6, 2).unwrap(),
        Decimal::parse("1.00", 6, 2).unwrap(),
        Decimal::parse("100.00", 6, 2).unwrap(),
    ];
    for pair in values.windows(2) {
        assert!(pair[0].encode() < pair[1].encode(), "{} must sort before {}", pair[0], pair[1]);
    }
    for value in values {
        assert_eq!(Decimal::decode(&value.encode(), 6, 2).unwrap(), value);
    }
    assert_eq!(Decimal::decode(&[0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 4, 2).unwrap().unscaled(), 0);
    assert!(matches!(Decimal::decode(&[0xff; 16], 2, 0), Err(ScalarError::PrecisionOverflow { .. })));
    assert!(matches!(Decimal::decode(&[0; 15], 2, 0), Err(ScalarError::InvalidLength { expected: 16, got: 15, .. })));
}

#[test]
fn tagged_json_roundtrips_without_loss_and_rejects_unknown_tags() {
    let values = [
        yydb_types::Value::Uuid(uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()),
        yydb_types::Value::Date(Date::parse_iso("2024-02-29").unwrap()),
        yydb_types::Value::Time(Time::parse_iso("23:59:59.999999999").unwrap()),
        yydb_types::Value::Timestamp(Timestamp::parse_iso("9999-12-31T23:59:59.999999Z").unwrap()),
        yydb_types::Value::Decimal(Decimal::parse("-1234.567890", 12, 6).unwrap()),
        yydb_types::Value::Bytes(vec![0x00, 0xff, 0x10, 0x80]),
    ];
    for value in values {
        let json = scalar::tagged_value_json(&value).expect("tagged");
        let decoded = scalar::tagged_value_from_json(&json).expect("object").expect("valid");
        assert_eq!(decoded, value);
    }
    assert!(scalar::tagged_value_json(&yydb_types::Value::Null).is_none());
    let document = serde_json::json!({ "$yydb": "decimal", "precision": 2, "scale": 0, "unscaled": "100" });
    assert!(matches!(scalar::tagged_value_from_json(&document).unwrap(), Err(ScalarError::Malformed { kind: "decimal", .. })));
    let odd_length = serde_json::json!({ "$yydb": "bytes", "hex": "abc" });
    assert!(matches!(scalar::tagged_value_from_json(&odd_length).unwrap(), Err(ScalarError::Malformed { kind: "bytes", .. })));
    let non_hex = serde_json::json!({ "$yydb": "bytes", "hex": "0g" });
    assert!(matches!(scalar::tagged_value_from_json(&non_hex).unwrap(), Err(ScalarError::Malformed { kind: "bytes", .. })));
    let bad_uuid = serde_json::json!({ "$yydb": "uuid", "value": "not-a-uuid" });
    assert!(matches!(scalar::tagged_value_from_json(&bad_uuid).unwrap(), Err(ScalarError::Malformed { kind: "uuid", .. })));
    let unknown = serde_json::json!({ "$yydb": "money" });
    assert!(scalar::tagged_value_from_json(&unknown).unwrap().is_err());
    let wrong_type = serde_json::json!({ "$yydb": "timestamp", "utcMicros": 1 });
    assert!(scalar::tagged_value_from_json(&wrong_type).unwrap().is_err());
}
