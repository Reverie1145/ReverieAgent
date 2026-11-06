//! Time helpers.
//!
//! Every persisted timestamp is stored as an RFC 3339 UTC string so that the
//! SQLite representation is human readable, lexicographically sortable and
//! immune to timezone drift.

use chrono::{DateTime, SecondsFormat, Utc};

/// Current wall-clock instant in UTC.
pub fn now() -> DateTime<Utc> {
    Utc::now()
}

/// Render an instant in the canonical storage format.
pub fn to_db(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Render "now" in the canonical storage format.
pub fn now_db() -> String {
    to_db(now())
}

/// Parse a stored timestamp, falling back to `now` when the value is missing or
/// malformed. The store never panics on a corrupt row.
pub fn from_db(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| now())
}

/// Parse an optional stored timestamp.
pub fn from_db_opt(value: Option<&str>) -> Option<DateTime<Utc>> {
    value.and_then(|v| DateTime::parse_from_rfc3339(v).ok()).map(|dt| dt.with_timezone(&Utc))
}

/// Milliseconds elapsed between two instants, saturating at zero.
pub fn elapsed_ms(start: DateTime<Utc>, end: DateTime<Utc>) -> i64 {
    (end - start).num_milliseconds().max(0)
}
