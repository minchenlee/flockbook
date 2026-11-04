//! Helpers shared by the parsers, ported from HerdrKit: ISO 8601 times, the 4 KB cut, Foundation-style pretty JSON,
//! Swift `String` semantics (`text`) and `JSONSerialization` casts (`json`).

pub mod json;
pub mod text;

use std::sync::OnceLock;

use chrono::NaiveDate;
use regex::Regex;
use serde_json::{Map, Value};

/// A point in time, UTC (Swift's `Date`). The Swift-port modules (resolver, status line) use it; the parsers keep
/// seconds since the epoch (`f64`).
pub type Date = chrono::DateTime<chrono::Utc>;

fn time_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"^([0-9]{4})-([0-9]{2})-([0-9]{2})T([0-9]{2}):([0-9]{2}):([0-9]{2})(\.[0-9]+)?(Z|[+-][0-9]{2}:?[0-9]{2})$")
            .unwrap()
    })
}

/// ISO 8601 with or without fractional seconds (`2026-10-09T01:02:03.456Z`), to the millisecond (further digits are
/// dropped, as Foundation's formatter does). Seconds since the Unix epoch.
pub fn parse_time(s: &str) -> Option<f64> {
    let c = time_re().captures(s)?;
    let n = |i: usize| c[i].parse::<u32>().ok();
    let ms = c.get(7).map(|f| {
        let digits: String = f.as_str()[1..].chars().chain("000".chars()).take(3).collect();
        digits.parse::<u32>().unwrap_or(0)
    });
    let local = NaiveDate::from_ymd_opt(c[1].parse().ok()?, n(2)?, n(3)?)?.and_hms_milli_opt(n(4)?, n(5)?, n(6)?, ms.unwrap_or(0))?;
    let zone = &c[8];
    let off: i64 = if zone == "Z" {
        0
    } else {
        let digits: String = zone[1..].chars().filter(char::is_ascii_digit).collect();
        let h: i64 = digits[..2].parse().ok()?;
        let m: i64 = digits[2..].parse().ok()?;
        if h > 23 || m > 59 {
            return None;
        }
        let v = h * 3600 + m * 60;
        if zone.starts_with('-') {
            -v
        } else {
            v
        }
    };
    let utc = local - chrono::Duration::seconds(off);
    Some(utc.and_utc().timestamp_millis() as f64 / 1000.0)
}

pub const MAX_DETAIL: usize = 4096;

/// Cuts to 4 KB of UTF-8 on a Character (grapheme cluster) boundary and appends "…".
pub fn cut(s: &str) -> String {
    if s.len() <= MAX_DETAIL {
        return s.to_string();
    }
    // Step back from byte 4096 to the start of a character.
    let mut i = MAX_DETAIL;
    while i > 0 && !text::is_boundary(s, i) {
        i -= 1;
    }
    format!("{}…", &s[..i])
}

/// `JSONSerialization` pretty JSON (`[.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]`): sorted keys,
/// `"key" : value`.
pub fn pretty_json(o: &Map<String, Value>) -> String {
    json::pretty_obj(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_formats() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0.0));
        assert_eq!(parse_time("2026-10-09T01:02:10Z"), Some(1791507730.0));
        assert_eq!(parse_time("2026-10-09T09:02:10+08:00"), Some(1791507730.0));
        assert_eq!(parse_time("2026-10-09T09:02:10+0800"), Some(1791507730.0));
        assert_eq!(parse_time("2026-10-09T01:02:10.5Z"), Some(1791507730.5));
        assert_eq!(parse_time("2026-10-09T01:02:10.123456Z"), Some(1791507730.123), "to the millisecond");
        assert_eq!(parse_time("garbage"), None);
        assert_eq!(parse_time("2026-13-09T01:02:10Z"), None);
        assert_eq!(parse_time("2026-10-09 01:02:10Z"), None, "a space for the T is not ISO 8601");
    }

    #[test]
    fn pretty_matches_foundation_style() {
        let v: Value = serde_json::from_str(r#"{"b":[1,2],"a":"/x","c":{},"d":2.0}"#).unwrap();
        let s = pretty_json(v.as_object().unwrap());
        assert_eq!(s, "{\n  \"a\" : \"/x\",\n  \"b\" : [\n    1,\n    2\n  ],\n  \"c\" : {\n\n  },\n  \"d\" : 2\n}");
    }
}
