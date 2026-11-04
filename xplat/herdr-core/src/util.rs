//! Small helpers shared by the parsers: ISO 8601 times, 4 KB cut, Swift-style pretty JSON.

use serde_json::{ser::Formatter, Value};
use std::io;

/// A point in time, UTC (Swift's `Date`). The Swift-port modules (resolver, status line) use it; the parsers keep
/// seconds since the epoch (`f64`).
pub type Date = chrono::DateTime<chrono::Utc>;

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// ISO 8601 with or without fractional seconds, `Z` or `±hh:mm`. Seconds since the Unix epoch.
pub fn parse_time(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !(b[10] == b'T' || b[10] == b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, sec) = (num(0, 4)?, num(5, 7)?, num(8, 10)?, num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut i = 19;
    let mut frac = 0.0;
    if b.get(i) == Some(&b'.') {
        let start = i + 1;
        i = start;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return None;
        }
        frac = format!("0.{}", &s[start..i]).parse().ok()?;
    }
    let off = match s.get(i..)? {
        "Z" | "z" => 0,
        "" => return None,
        r => {
            let sign = match r.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let digits: String = r[1..].chars().filter(|c| *c != ':').collect();
            if digits.len() != 4 {
                return None;
            }
            sign * (digits[..2].parse::<i64>().ok()? * 3600 + digits[2..].parse::<i64>().ok()? * 60)
        }
    };
    let days = days_from_civil(y, mo, d);
    Some((days * 86400 + h * 3600 + mi * 60 + sec - off) as f64 + frac)
}

pub const MAX_DETAIL: usize = 4096;

/// Cuts to 4 KB of UTF-8 on a character boundary and appends "…".
pub fn cut(s: &str) -> String {
    if s.len() <= MAX_DETAIL {
        return s.to_string();
    }
    let mut i = MAX_DETAIL;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    format!("{}…", &s[..i])
}

/// Pretty JSON, sorted keys (serde_json's map is a BTreeMap), `"key" : value` like Foundation prints it.
pub fn pretty_json(o: &serde_json::Map<String, Value>) -> String {
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, SwiftPretty::default());
    if serde::Serialize::serialize(o, &mut ser).is_err() {
        return String::new();
    }
    String::from_utf8(out).unwrap_or_default()
}

#[derive(Default)]
struct SwiftPretty {
    depth: usize,
    has_value: bool,
}

impl SwiftPretty {
    fn indent<W: ?Sized + io::Write>(&self, w: &mut W) -> io::Result<()> {
        for _ in 0..self.depth {
            w.write_all(b"  ")?;
        }
        Ok(())
    }
}

impl Formatter for SwiftPretty {
    fn begin_array<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.depth += 1;
        self.has_value = false;
        w.write_all(b"[")
    }
    fn end_array<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.depth -= 1;
        if self.has_value {
            w.write_all(b"\n")?;
            self.indent(w)?;
        }
        self.has_value = true;
        w.write_all(b"]")
    }
    fn begin_array_value<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        w.write_all(if first { b"\n" } else { b",\n" })?;
        self.indent(w)
    }
    fn end_array_value<W: ?Sized + io::Write>(&mut self, _w: &mut W) -> io::Result<()> {
        self.has_value = true;
        Ok(())
    }
    fn begin_object<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.depth += 1;
        self.has_value = false;
        w.write_all(b"{")
    }
    fn end_object<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.depth -= 1;
        if self.has_value {
            w.write_all(b"\n")?;
            self.indent(w)?;
        }
        self.has_value = true;
        w.write_all(b"}")
    }
    fn begin_object_key<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        w.write_all(if first { b"\n" } else { b",\n" })?;
        self.indent(w)
    }
    fn begin_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        w.write_all(b" : ")
    }
    fn end_object_value<W: ?Sized + io::Write>(&mut self, _w: &mut W) -> io::Result<()> {
        self.has_value = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_formats() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0.0));
        assert_eq!(parse_time("2026-10-09T01:02:10Z"), Some(1791507730.0));
        assert_eq!(parse_time("2026-10-09T09:02:10+08:00"), Some(1791507730.0));
        assert_eq!(parse_time("2026-10-09T01:02:10.5Z"), Some(1791507730.5));
        assert_eq!(parse_time("garbage"), None);
        assert_eq!(parse_time("2026-13-09T01:02:10Z"), None);
    }

    #[test]
    fn pretty_matches_foundation_style() {
        let v: Value = serde_json::from_str(r#"{"b":[1,2],"a":"/x","c":{}}"#).unwrap();
        let s = pretty_json(v.as_object().unwrap());
        assert_eq!(s, "{\n  \"a\" : \"/x\",\n  \"b\" : [\n    1,\n    2\n  ],\n  \"c\" : {}\n}");
    }
}
