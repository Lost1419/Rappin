//! Timestamp encoding shared by transport DTOs and durable event evidence.

use jiff::{fmt::temporal::DateTimePrinter, tz::Offset, Timestamp};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

/// Parse a Yapin RFC 3339 timestamp at nanosecond precision.
///
/// ```rust
/// let time = rappin::parse_timestamp("2026-11-18T12:34:56.123456789Z")?;
/// assert_eq!(time.subsec_nanosecond(), 123_456_789);
/// # Ok::<(), rappin::Error>(())
/// ```
///
/// Leap seconds are rejected because the SDK timestamp uses Unix time.
pub fn parse_timestamp(value: &str) -> crate::Result<Timestamp> {
    decode(value).map_err(|reason| crate::Error::validation("timestamp", reason))
}

fn decode(value: &str) -> Result<Timestamp, &'static str> {
    const INVALID: &str = "expected an RFC 3339 timestamp with at most nine fractional digits";
    let bytes = value.as_bytes();
    let suffix = bytes.get(19..).ok_or(INVALID)?;
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't' | b' ')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return Err(INVALID);
    }
    if &bytes[17..19] == b"60" {
        return Err("leap seconds are not representable in Unix time");
    }
    let offset = if let Some(fraction) = suffix.strip_prefix(b".") {
        let digits = fraction.iter().take_while(|b| b.is_ascii_digit()).count();
        if !(1..=9).contains(&digits) {
            return Err(INVALID);
        }
        &fraction[digits..]
    } else {
        suffix
    };
    match offset {
        [b'Z' | b'z'] => {}
        [b'+' | b'-', tens @ b'0'..=b'2', units @ b'0'..=b'9', b':', b'0'..=b'5', b'0'..=b'9']
            if (*tens - b'0') * 10 + *units - b'0' <= 23 => {}
        _ => return Err(INVALID),
    }
    value.parse().map_err(|_| INVALID)
}

fn precision(timestamp: &Timestamp) -> u8 {
    let nanos = timestamp.subsec_nanosecond();
    // Preserve canonical JSON bytes used by durable fingerprints.
    if nanos == 0 {
        0
    } else if nanos % 1_000_000 == 0 {
        3
    } else if nanos % 1_000 == 0 {
        6
    } else {
        9
    }
}

pub(crate) fn query(timestamp: Timestamp) -> String {
    DateTimePrinter::new()
        .precision(Some(precision(&timestamp)))
        .timestamp_with_offset_to_string(&timestamp, Offset::UTC)
}

pub(crate) fn serialize<S: Serializer>(
    timestamp: &Timestamp,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let precision = usize::from(precision(timestamp));
    serializer.collect_str(&format_args!("{timestamp:.precision$}"))
}

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Timestamp, D::Error> {
    struct TimestampVisitor;

    impl de::Visitor<'_> for TimestampVisitor {
        type Value = Timestamp;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("an RFC 3339 timestamp")
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<Timestamp, E> {
            decode(value).map_err(E::custom)
        }
    }

    deserializer.deserialize_str(TimestampVisitor)
}

pub(crate) mod optional {
    use super::*;

    #[derive(Deserialize)]
    #[serde(transparent)]
    struct WireTimestamp(#[serde(deserialize_with = "super::deserialize")] Timestamp);

    pub(crate) fn serialize<S: Serializer>(
        timestamp: &Option<Timestamp>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        struct BorrowedTimestamp<'a>(&'a Timestamp);

        impl Serialize for BorrowedTimestamp<'_> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                super::serialize(self.0, serializer)
            }
        }

        match timestamp {
            Some(timestamp) => serializer.serialize_some(&BorrowedTimestamp(timestamp)),
            None => serializer.serialize_none(),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Timestamp>, D::Error> {
        Ok(Option::<WireTimestamp>::deserialize(deserializer)?.map(|timestamp| timestamp.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_timestamps_that_would_lose_precision_or_offset() {
        for input in [
            "2016-12-31T23:59:60Z",
            "2026-11-18T12:34:56.1234567891Z",
            "2026-11-18T12:34:56",
            "2026-11-18T12:34:56+25:00",
            "2026-11-18T12:34:56Z[UTC]",
        ] {
            assert!(decode(input).is_err(), "{input}");
        }
    }
}
