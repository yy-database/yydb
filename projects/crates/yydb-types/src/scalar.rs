//! Exact calendar, clock, instant, and fixed-point decimal scalars.
//!
//! Every type here has one canonical binary encoding with a fixed byte width
//! whose lexicographic byte order matches value order, so encoded values can be
//! compared without decoding. Text forms are the ISO-8601 subset accepted by
//! [`Date::parse_iso`], [`Time::parse_iso`], and [`Timestamp::parse_iso`].
//!
//! Precision rules are explicit rather than rounded:
//!
//! - [`Date`] covers years `0001..=9999`.
//! - [`Time`] is a wall clock reading with nanosecond resolution.
//! - [`Timestamp`] is an absolute UTC instant with microsecond resolution.
//!   Local offsets other than `Z` / `+00:00` are rejected.
//! - [`Decimal`] is an exact `unscaled × 10^-scale` integer with declared
//!   precision `1..=38` and scale `0..=precision`. Values that need more
//!   fractional digits or more total digits than declared are rejected.

use std::{cmp::Ordering, fmt};

/// Nanoseconds per second.
pub const NANOS_PER_SECOND: i64 = 1_000_000_000;
/// Nanoseconds per day.
pub const NANOS_PER_DAY: i64 = 86_400 * NANOS_PER_SECOND;
/// Microseconds per second.
pub const MICROS_PER_SECOND: i64 = 1_000_000;
/// Microseconds per day.
pub const MICROS_PER_DAY: i64 = 86_400 * MICROS_PER_SECOND;
/// Largest decimal precision supported by [`Decimal`] (`i128` magnitude).
pub const DECIMAL_MAX_PRECISION: u8 = 38;

/// Rejection raised by a scalar parser, decoder, or constructor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScalarError {
    /// Text does not match the canonical format for its kind.
    Malformed {
        /// Scalar kind (`date`, `time`, `timestamp`, or `decimal`).
        kind: &'static str,
        /// Rejected input.
        input: String,
    },
    /// A numeric component is outside the supported range.
    OutOfRange {
        /// Scalar kind.
        kind: &'static str,
        /// Human-readable range detail.
        detail: String,
    },
    /// A timestamp text carried a zone offset other than UTC.
    UnsupportedUtcOffset {
        /// Rejected offset text.
        offset: String,
    },
    /// The value needs finer precision than the declared resolution.
    PrecisionLoss {
        /// Scalar kind.
        kind: &'static str,
        /// Human-readable detail.
        detail: String,
    },
    /// Decimal precision is outside `1..=38`.
    InvalidPrecision {
        /// Rejected precision.
        precision: u8,
    },
    /// Decimal scale exceeds its precision.
    InvalidScale {
        /// Declared precision.
        precision: u8,
        /// Rejected scale.
        scale: u8,
    },
    /// Decimal value needs more digits than its precision allows.
    PrecisionOverflow {
        /// Declared precision.
        precision: u8,
        /// Rejected unscaled digit text.
        digits: String,
    },
    /// Decimal text carried more fractional digits than its scale.
    ScaleOverflow {
        /// Declared scale.
        scale: u8,
        /// Rejected fractional digits.
        fraction: String,
    },
    /// Encoded bytes do not have the exact fixed width of the value.
    InvalidLength {
        /// Scalar kind.
        kind: &'static str,
        /// Required byte length.
        expected: usize,
        /// Supplied byte length.
        got: usize,
    },
}

impl fmt::Display for ScalarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed { kind, input } => write!(f, "invalid {kind}: `{input}`"),
            Self::OutOfRange { kind, detail } => write!(f, "{kind} out of range: {detail}"),
            Self::UnsupportedUtcOffset { offset } => {
                write!(f, "timestamp offset `{offset}` is unsupported: only UTC (`Z` or `+00:00`) is accepted")
            }
            Self::PrecisionLoss { kind, detail } => write!(f, "{kind} precision loss: {detail}"),
            Self::InvalidPrecision { precision } => {
                write!(f, "decimal precision {precision} is outside 1..={DECIMAL_MAX_PRECISION}")
            }
            Self::InvalidScale { precision, scale } => {
                write!(f, "decimal scale {scale} exceeds precision {precision}")
            }
            Self::PrecisionOverflow { precision, digits } => {
                write!(f, "decimal digits `{digits}` exceed precision {precision}")
            }
            Self::ScaleOverflow { scale, fraction } => {
                write!(f, "decimal fraction `.{}` exceeds scale {scale}", fraction)
            }
            Self::InvalidLength { kind, expected, got } => {
                write!(f, "{kind} encoding expects {expected} bytes, got {got}")
            }
        }
    }
}

impl std::error::Error for ScalarError {}

impl ScalarError {
    /// Scalar kind label (`date`, `time`, `timestamp`, `decimal`, or `scalar`).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Malformed { kind, .. }
            | Self::OutOfRange { kind, .. }
            | Self::PrecisionLoss { kind, .. }
            | Self::InvalidLength { kind, .. } => kind,
            Self::UnsupportedUtcOffset { .. } => "timestamp",
            Self::InvalidPrecision { .. }
            | Self::InvalidScale { .. }
            | Self::PrecisionOverflow { .. }
            | Self::ScaleOverflow { .. } => "decimal",
        }
    }
}

impl From<ScalarError> for crate::Error {
    fn from(error: ScalarError) -> Self {
        Self::Scalar { kind: error.kind(), message: error.to_string() }
    }
}

const fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

const fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since `1970-01-01` for a proleptic Gregorian civil date.
///
/// Port of Howard Hinnant's `days_from_civil`; valid for the supported range.
const fn days_from_civil(year: i32, month: u32, day: u32) -> i32 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = (year - era * 400) as u32;
    let month_prime = (month as i32 + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day as i32 - 1;
    let day_of_era = year_of_era as i32 * 365 + year_of_era as i32 / 4 - year_of_era as i32 / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Inverse of [`days_from_civil`].
const fn civil_from_days(days: i32) -> (i32, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 { shifted } else { shifted - 146_096 } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u32;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i32 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 { month_prime + 3 } else { month_prime - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

fn flip_sign_u128_to_bytes(value: u128) -> [u8; 16] {
    (value ^ (1_u128 << 127)).to_be_bytes()
}

fn flip_sign_bytes_to_u128(bytes: [u8; 16]) -> u128 {
    u128::from_be_bytes(bytes) ^ (1_u128 << 127)
}

/// Ordered 8-byte encoding of a signed integer: sign bit flipped, big endian.
fn encode_i64_ordered(value: i64) -> [u8; 8] {
    ((value as u64) ^ (1_u64 << 63)).to_be_bytes()
}

fn decode_i64_ordered(bytes: [u8; 8]) -> i64 {
    (u64::from_be_bytes(bytes) ^ (1_u64 << 63)) as i64
}

/// Calendar date in the proleptic Gregorian calendar, no time and no zone.
///
/// Stored as `i32` days since `1970-01-01`. The supported range is
/// `0001-01-01..=9999-12-31`; anything outside is rejected instead of clamped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    days: i32,
}

impl Date {
    /// Smallest supported date (`0001-01-01`).
    pub const MIN: Self = Self { days: days_from_civil(1, 1, 1) };
    /// Largest supported date (`9999-12-31`).
    pub const MAX: Self = Self { days: days_from_civil(9999, 12, 31) };

    /// Creates a date from civil year, month, and day.
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Result<Self, ScalarError> {
        if !(1..=9999).contains(&year) {
            return Err(ScalarError::OutOfRange {
                kind: "date",
                detail: format!("year {year} is outside 0001..=9999"),
            });
        }
        if !(1..=12).contains(&month) {
            return Err(ScalarError::OutOfRange {
                kind: "date",
                detail: format!("month {month} is outside 1..=12"),
            });
        }
        let maximum_day = days_in_month(year, month);
        if day == 0 || day > maximum_day {
            return Err(ScalarError::OutOfRange {
                kind: "date",
                detail: format!("day {day} is outside 1..={maximum_day} for {year:04}-{month:02}"),
            });
        }
        Ok(Self { days: days_from_civil(year, month, day) })
    }

    /// Creates a date from days since the UNIX epoch within the supported range.
    pub fn from_days(days: i32) -> Result<Self, ScalarError> {
        if !(Self::MIN.days..=Self::MAX.days).contains(&days) {
            return Err(ScalarError::OutOfRange {
                kind: "date",
                detail: format!("{days} days since 1970-01-01 is outside 0001-01-01..=9999-12-31"),
            });
        }
        Ok(Self { days })
    }

    /// Days since `1970-01-01`.
    pub const fn days(self) -> i32 {
        self.days
    }

    /// Civil `(year, month, day)`.
    pub fn to_ymd(self) -> (i32, u32, u32) {
        civil_from_days(self.days)
    }

    /// Parses the canonical `YYYY-MM-DD` form.
    pub fn parse_iso(text: &str) -> Result<Self, ScalarError> {
        let malformed = || ScalarError::Malformed { kind: "date", input: text.to_owned() };
        if !text.is_ascii() {
            return Err(malformed());
        }
        let bytes = text.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return Err(malformed());
        }
        let year = parse_digits(&bytes[0..4]).map(|value| value as i32).ok_or_else(malformed)?;
        let month = parse_digits(&bytes[5..7]).ok_or_else(malformed)?;
        let day = parse_digits(&bytes[8..10]).ok_or_else(malformed)?;
        Self::from_ymd(year, month, day)
    }

    /// Canonical `YYYY-MM-DD` form.
    pub fn to_iso(self) -> String {
        let (year, month, day) = self.to_ymd();
        format!("{year:04}-{month:02}-{day:02}")
    }

    /// Fixed-width 4-byte order-preserving encoding.
    pub fn encode(self) -> [u8; 4] {
        ((self.days as u32) ^ (1_u32 << 31)).to_be_bytes()
    }

    /// Decodes a 4-byte encoding produced by [`Self::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Self, ScalarError> {
        let bytes: [u8; 4] = bytes.try_into().map_err(|_| ScalarError::InvalidLength {
            kind: "date",
            expected: 4,
            got: bytes.len(),
        })?;
        Self::from_days((u32::from_be_bytes(bytes) ^ (1_u32 << 31)) as i32)
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_iso())
    }
}

/// Wall-clock time of day with nanosecond resolution and no zone.
///
/// Stored as `i64` nanoseconds since midnight in `0..NANOS_PER_DAY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time {
    nanos: i64,
}

impl Time {
    /// First instant of the day (`00:00:00`).
    pub const MIN: Self = Self { nanos: 0 };
    /// Last representable instant (`23:59:59.999999999`).
    pub const MAX: Self = Self { nanos: NANOS_PER_DAY - 1 };

    /// Creates a time from hour, minute, second, and nanosecond components.
    pub fn from_hms_nano(hour: u32, minute: u32, second: u32, nanosecond: u32) -> Result<Self, ScalarError> {
        if hour > 23 || minute > 59 || second > 59 {
            return Err(ScalarError::OutOfRange {
                kind: "time",
                detail: format!("{hour:02}:{minute:02}:{second:02} is not a wall-clock time"),
            });
        }
        if nanosecond >= NANOS_PER_SECOND as u32 {
            return Err(ScalarError::OutOfRange {
                kind: "time",
                detail: format!("{nanosecond} nanoseconds is outside 0..1_000_000_000"),
            });
        }
        let nanos = (hour as i64 * 3_600 + minute as i64 * 60 + second as i64) * NANOS_PER_SECOND + nanosecond as i64;
        Ok(Self { nanos })
    }

    /// Creates a time from nanoseconds since midnight.
    pub fn from_nanos(nanos: i64) -> Result<Self, ScalarError> {
        if !(0..NANOS_PER_DAY).contains(&nanos) {
            return Err(ScalarError::OutOfRange {
                kind: "time",
                detail: format!("{nanos} nanoseconds since midnight is outside one day"),
            });
        }
        Ok(Self { nanos })
    }

    /// Nanoseconds since midnight.
    pub const fn nanos(self) -> i64 {
        self.nanos
    }

    /// `(hour, minute, second, nanosecond)`.
    pub fn to_hms_nano(self) -> (u32, u32, u32, u32) {
        let seconds = (self.nanos / NANOS_PER_SECOND) as u32;
        let nanosecond = (self.nanos % NANOS_PER_SECOND) as u32;
        (seconds / 3_600, (seconds % 3_600) / 60, seconds % 60, nanosecond)
    }

    /// Parses `HH:MM:SS` with up to nine optional fractional digits.
    pub fn parse_iso(text: &str) -> Result<Self, ScalarError> {
        let malformed = || ScalarError::Malformed { kind: "time", input: text.to_owned() };
        if !text.is_ascii() {
            return Err(malformed());
        }
        let bytes = text.as_bytes();
        if bytes.len() < 8 || bytes[2] != b':' || bytes[5] != b':' {
            return Err(malformed());
        }
        let hour = parse_digits(&bytes[0..2]).ok_or_else(malformed)?;
        let minute = parse_digits(&bytes[3..5]).ok_or_else(malformed)?;
        let second = parse_digits(&bytes[6..8]).ok_or_else(malformed)?;
        let nanosecond = if bytes.len() == 8 {
            0
        }
        else {
            if bytes[8] != b'.' {
                return Err(malformed());
            }
            let fraction = &bytes[9..];
            if fraction.is_empty() || fraction.len() > 9 {
                return Err(malformed());
            }
            let digits = parse_digits(fraction).ok_or_else(malformed)?;
            digits * 10_u32.pow(9 - fraction.len() as u32)
        };
        Self::from_hms_nano(hour, minute, second, nanosecond)
    }

    /// Canonical `HH:MM:SS[.ffffff]` form with trailing zeros trimmed.
    pub fn to_iso(self) -> String {
        let (hour, minute, second, nanosecond) = self.to_hms_nano();
        let mut out = format!("{hour:02}:{minute:02}:{second:02}");
        if nanosecond != 0 {
            let fraction = format!("{nanosecond:09}");
            out.push('.');
            out.push_str(fraction.trim_end_matches('0'));
        }
        out
    }

    /// Fixed-width 8-byte order-preserving encoding.
    pub fn encode(self) -> [u8; 8] {
        encode_i64_ordered(self.nanos)
    }

    /// Decodes an 8-byte encoding produced by [`Self::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Self, ScalarError> {
        let bytes: [u8; 8] = bytes.try_into().map_err(|_| ScalarError::InvalidLength {
            kind: "time",
            expected: 8,
            got: bytes.len(),
        })?;
        Self::from_nanos(decode_i64_ordered(bytes))
    }
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_iso())
    }
}

/// Absolute instant in UTC with microsecond resolution.
///
/// Stored as `i64` microseconds since the UNIX epoch. The supported range is
/// `0001-01-01T00:00:00Z..=9999-12-31T23:59:59.999999Z`. The zone is always
/// UTC: parsers accept `Z` or `+00:00` only and reject every other offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp {
    micros: i64,
}

impl Timestamp {
    /// Earliest supported instant.
    pub const MIN: Self = Self { micros: Date::MIN.days as i64 * MICROS_PER_DAY };
    /// Latest supported instant.
    pub const MAX: Self = Self { micros: Date::MAX.days as i64 * MICROS_PER_DAY + MICROS_PER_DAY - 1 };

    /// Creates an instant from microseconds since the UNIX epoch.
    pub fn from_unix_micros(micros: i64) -> Result<Self, ScalarError> {
        if !(Self::MIN.micros..=Self::MAX.micros).contains(&micros) {
            return Err(ScalarError::OutOfRange {
                kind: "timestamp",
                detail: format!("{micros} microseconds since 1970-01-01T00:00:00Z is outside the supported range"),
            });
        }
        Ok(Self { micros })
    }

    /// Combines a date and a time; sub-microsecond components are rejected.
    pub fn from_parts(date: Date, time: Time) -> Result<Self, ScalarError> {
        if time.nanos % 1_000 != 0 {
            return Err(ScalarError::PrecisionLoss {
                kind: "timestamp",
                detail: format!("time `{time}` carries sub-microsecond digits"),
            });
        }
        Self::from_unix_micros(date.days as i64 * MICROS_PER_DAY + time.nanos / 1_000)
    }

    /// Microseconds since the UNIX epoch.
    pub const fn unix_micros(self) -> i64 {
        self.micros
    }

    /// Splits the instant into its UTC date and wall-clock time.
    pub fn to_parts(self) -> (Date, Time) {
        let days = self.micros.div_euclid(MICROS_PER_DAY);
        let micros_of_day = self.micros.rem_euclid(MICROS_PER_DAY);
        let date = Date { days: days as i32 };
        let time = Time { nanos: micros_of_day * 1_000 };
        (date, time)
    }

    /// Parses `YYYY-MM-DDTHH:MM:SS[.ffffff]Z` (or `+00:00`).
    pub fn parse_iso(text: &str) -> Result<Self, ScalarError> {
        let malformed = || ScalarError::Malformed { kind: "timestamp", input: text.to_owned() };
        if !text.is_ascii() {
            return Err(malformed());
        }
        let bytes = text.as_bytes();
        if bytes.len() < 20 || bytes[10] != b'T' && bytes[10] != b't' {
            return Err(malformed());
        }
        let date = Date::parse_iso(&text[..10])?;
        let rest = &text[11..];
        let clock = if let Some(clock) = rest.strip_suffix('Z') {
            clock
        }
        else if let Some(clock) = rest.strip_suffix("+00:00") {
            clock
        }
        else if let Some(index) = rest.rfind('+').or_else(|| rest.rfind('-')) {
            return Err(ScalarError::UnsupportedUtcOffset { offset: rest[index..].to_owned() });
        }
        else {
            return Err(malformed());
        };
        let time = Time::parse_iso(clock)?;
        if time.nanos % 1_000 != 0 {
            return Err(ScalarError::PrecisionLoss {
                kind: "timestamp",
                detail: format!("time `{clock}` carries sub-microsecond digits"),
            });
        }
        Self::from_parts(date, time)
    }

    /// Canonical `YYYY-MM-DDTHH:MM:SS[.ffffff]Z` form.
    pub fn to_iso(self) -> String {
        let (date, time) = self.to_parts();
        let (hour, minute, second, nanosecond) = time.to_hms_nano();
        let micros = nanosecond / 1_000;
        let mut out = format!("{}T{hour:02}:{minute:02}:{second:02}", date.to_iso());
        if micros != 0 {
            let fraction = format!("{micros:06}");
            out.push('.');
            out.push_str(fraction.trim_end_matches('0'));
        }
        out.push('Z');
        out
    }

    /// Fixed-width 8-byte order-preserving encoding.
    pub fn encode(self) -> [u8; 8] {
        encode_i64_ordered(self.micros)
    }

    /// Decodes an 8-byte encoding produced by [`Self::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Self, ScalarError> {
        let bytes: [u8; 8] = bytes.try_into().map_err(|_| ScalarError::InvalidLength {
            kind: "timestamp",
            expected: 8,
            got: bytes.len(),
        })?;
        Self::from_unix_micros(decode_i64_ordered(bytes))
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_iso())
    }
}

/// Exact fixed-point decimal with declared precision and scale.
///
/// The value is `unscaled × 10^-scale` and is stored as an `i128`; comparison
/// never converts through floating point. Precision is the maximum number of
/// digits `unscaled` may need (`1..=38`) and scale the exact number of
/// fractional digits (`0..=precision`). Inputs that exceed either bound are
/// rejected instead of rounded or truncated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Decimal {
    unscaled: i128,
    precision: u8,
    scale: u8,
}

impl Decimal {
    /// Creates a decimal from an unscaled integer and its declared shape.
    pub fn new(unscaled: i128, precision: u8, scale: u8) -> Result<Self, ScalarError> {
        if precision == 0 || precision > DECIMAL_MAX_PRECISION {
            return Err(ScalarError::InvalidPrecision { precision });
        }
        if scale > precision {
            return Err(ScalarError::InvalidScale { precision, scale });
        }
        let magnitude = unscaled.unsigned_abs();
        let limit = 10_u128.pow(precision as u32);
        if magnitude >= limit {
            return Err(ScalarError::PrecisionOverflow {
                precision,
                digits: unscaled.to_string(),
            });
        }
        Ok(Self { unscaled, precision, scale })
    }

    /// Parses exact decimal text into the declared shape.
    ///
    /// Accepted forms are an optional sign, at least one digit, and an optional
    /// `.` fraction. Exponents, whitespace, and thousands separators are
    /// rejected. `-0` and `-0.00` normalize to zero.
    pub fn parse(text: &str, precision: u8, scale: u8) -> Result<Self, ScalarError> {
        let malformed = || ScalarError::Malformed { kind: "decimal", input: text.to_owned() };
        if precision == 0 || precision > DECIMAL_MAX_PRECISION {
            return Err(ScalarError::InvalidPrecision { precision });
        }
        if scale > precision {
            return Err(ScalarError::InvalidScale { precision, scale });
        }
        let bytes = text.as_bytes();
        let mut index = 0;
        let negative = match bytes.first() {
            Some(b'-') => {
                index = 1;
                true
            }
            Some(b'+') => {
                index = 1;
                false
            }
            _ => false,
        };
        let int_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        let integer = &text[int_start..index];
        let mut fraction = "";
        if index < bytes.len() && bytes[index] == b'.' {
            let fraction_start = index + 1;
            index = fraction_start;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            fraction = &text[fraction_start..index];
            if fraction.is_empty() {
                return Err(malformed());
            }
        }
        if index != bytes.len() || (integer.is_empty() && fraction.is_empty()) {
            return Err(malformed());
        }
        if fraction.len() > scale as usize {
            return Err(ScalarError::ScaleOverflow { scale, fraction: fraction.to_owned() });
        }
        let mut digits = String::with_capacity(integer.len() + scale as usize);
        digits.push_str(integer);
        digits.push_str(fraction);
        digits.extend(std::iter::repeat_n('0', scale as usize - fraction.len()));
        let magnitude = digits.parse::<i128>().map_err(|_| ScalarError::PrecisionOverflow {
            precision,
            digits: digits.clone(),
        })?;
        let unscaled = if negative { -magnitude } else { magnitude };
        Self::new(unscaled, precision, scale)
    }

    /// Declared precision.
    pub const fn precision(self) -> u8 {
        self.precision
    }

    /// Declared scale.
    pub const fn scale(self) -> u8 {
        self.scale
    }

    /// Exact unscaled integer.
    pub const fn unscaled(self) -> i128 {
        self.unscaled
    }

    /// Exact canonical text of this value at its declared scale.
    pub fn to_plain_string(self) -> String {
        let negative = self.unscaled < 0;
        let magnitude = self.unscaled.unsigned_abs().to_string();
        let scale = self.scale as usize;
        let mut digits = magnitude;
        if scale > 0 {
            if digits.len() <= scale {
                let padding = scale + 1 - digits.len();
                digits.insert_str(0, &"0".repeat(padding));
            }
            let split = digits.len() - scale;
            digits.insert(split, '.');
        }
        if negative && self.unscaled != 0 {
            digits.insert(0, '-');
        }
        digits
    }

    /// Exact comparison across scales without floating point.
    pub fn compare(self, other: Self) -> Ordering {
        if self.scale == other.scale {
            return self.unscaled.cmp(&other.unscaled);
        }
        let negative = self.unscaled < 0;
        let other_negative = other.unscaled < 0;
        if negative != other_negative {
            return if negative { Ordering::Less } else { Ordering::Greater };
        }
        let target = self.scale.max(other.scale);
        let left = self.aligned_digits(target);
        let right = other.aligned_digits(target);
        let left = left.trim_start_matches('0');
        let right = right.trim_start_matches('0');
        let ordering = left.len().cmp(&right.len()).then_with(|| left.cmp(right));
        if negative {
            ordering.reverse()
        }
        else {
            ordering
        }
    }

    fn aligned_digits(self, scale: u8) -> String {
        if scale <= self.scale {
            return self.unscaled.unsigned_abs().to_string();
        }
        let mut digits = self.unscaled.unsigned_abs().to_string();
        digits.extend(std::iter::repeat_n('0', (scale - self.scale) as usize));
        digits
    }

    /// Fixed-width 16-byte order-preserving encoding of the unscaled integer.
    ///
    /// Precision and scale are catalog metadata, not part of the value bytes,
    /// so decoders must be given the declared shape.
    pub fn encode(self) -> [u8; 16] {
        flip_sign_u128_to_bytes(self.unscaled as u128)
    }

    /// Decodes 16 bytes using the declared precision and scale.
    pub fn decode(bytes: &[u8], precision: u8, scale: u8) -> Result<Self, ScalarError> {
        let bytes: [u8; 16] = bytes.try_into().map_err(|_| ScalarError::InvalidLength {
            kind: "decimal",
            expected: 16,
            got: bytes.len(),
        })?;
        Self::new(flip_sign_bytes_to_u128(bytes) as i128, precision, scale)
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_plain_string())
    }
}

fn parse_digits(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut value = 0_u32;
    for byte in bytes {
        value = value.checked_mul(10)?.checked_add(u32::from(byte - b'0'))?;
    }
    Some(value)
}

/// JSON member marking a tagged scalar object on query-row and wire surfaces.
pub const TAGGED_JSON_KEY: &str = "$yydb";

/// Tagged JSON form for temporal and decimal values.
///
/// Returns `None` for every other [`crate::Value`] variant so callers keep
/// their existing encoding for booleans, integers, and text.
pub fn tagged_value_json(value: &crate::Value) -> Option<serde_json::Value> {
    match value {
        crate::Value::Date(date) => Some(serde_json::json!({ "$yydb": "date", "days": date.days() })),
        crate::Value::Time(time) => Some(serde_json::json!({ "$yydb": "time", "nanos": time.nanos() })),
        crate::Value::Timestamp(timestamp) => Some(serde_json::json!({
            "$yydb": "timestamp",
            "utcMicros": timestamp.unix_micros().to_string(),
        })),
        crate::Value::Decimal(decimal) => Some(serde_json::json!({
            "$yydb": "decimal",
            "precision": decimal.precision(),
            "scale": decimal.scale(),
            "unscaled": decimal.unscaled().to_string(),
        })),
        _ => None,
    }
}

/// Parses a tagged JSON object produced by [`tagged_value_json`].
///
/// Returns `None` when `json` is not an object carrying [`TAGGED_JSON_KEY`].
/// Unknown tags and malformed members return `Some(Err(..))` so callers can
/// reject them instead of silently degrading to a string or null.
pub fn tagged_value_from_json(json: &serde_json::Value) -> Option<Result<crate::Value, ScalarError>> {
    let tag = json.get(TAGGED_JSON_KEY)?;
    Some(parse_tagged_value(json, tag))
}

fn parse_tagged_value(json: &serde_json::Value, tag: &serde_json::Value) -> Result<crate::Value, ScalarError> {
    let malformed = |kind: &'static str| ScalarError::Malformed { kind, input: json.to_string() };
    let Some(tag) = tag.as_str() else {
        return Err(malformed("scalar"));
    };
    let object = json.as_object().ok_or_else(|| malformed("scalar"))?;
    match tag {
        "date" => {
            let days = object.get("days").and_then(serde_json::Value::as_i64).ok_or_else(|| malformed("date"))?;
            let days = i32::try_from(days).map_err(|_| malformed("date"))?;
            Date::from_days(days).map(crate::Value::Date).map_err(|error| mismatch("date", error, json))
        }
        "time" => {
            let nanos = object.get("nanos").and_then(serde_json::Value::as_i64).ok_or_else(|| malformed("time"))?;
            Time::from_nanos(nanos).map(crate::Value::Time).map_err(|error| mismatch("time", error, json))
        }
        "timestamp" => {
            let micros = object
                .get("utcMicros")
                .and_then(serde_json::Value::as_str)
                .and_then(|text| text.parse::<i64>().ok())
                .ok_or_else(|| malformed("timestamp"))?;
            Timestamp::from_unix_micros(micros)
                .map(crate::Value::Timestamp)
                .map_err(|error| mismatch("timestamp", error, json))
        }
        "decimal" => {
            let precision = object
                .get("precision")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| u8::try_from(value).ok())
                .ok_or_else(|| malformed("decimal"))?;
            let scale = object
                .get("scale")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| u8::try_from(value).ok())
                .ok_or_else(|| malformed("decimal"))?;
            let unscaled = object
                .get("unscaled")
                .and_then(serde_json::Value::as_str)
                .and_then(|text| text.parse::<i128>().ok())
                .ok_or_else(|| malformed("decimal"))?;
            Decimal::new(unscaled, precision, scale)
                .map(crate::Value::Decimal)
                .map_err(|error| mismatch("decimal", error, json))
        }
        other => Err(ScalarError::Malformed { kind: "scalar", input: format!("unknown tag `{other}`") }),
    }
}

fn mismatch(kind: &'static str, error: ScalarError, json: &serde_json::Value) -> ScalarError {
    match error {
        ScalarError::Malformed { .. } => error,
        _ => ScalarError::Malformed { kind, input: format!("{json}: {error}") },
    }
}
