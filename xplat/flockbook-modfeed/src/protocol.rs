//! Swift: `Protocol.swift`. The sidecar's output: JSON Lines (one object per line). The first line is a `header`
//! with the protocol version; the last is an `end`. The contract is the mod's docs/protocol.md.

use serde::Serialize;
use serde_json::Value;

/// Bumped on any change a mod must know about. The mod refuses a sidecar with another number.
pub const PROTOCOL_VERSION: i64 = 1;
pub const SIDECAR_VERSION: &str = "0.1.0";

/// Collects output lines; `text` prints them. Keys are sorted (serde_json's map is ordered by key) and slashes are
/// not escaped, as Swift's `.sortedKeys, .withoutEscapingSlashes`.
#[derive(Default, Debug)]
pub struct JsonLines {
    pub lines: Vec<String>,
}

impl JsonLines {
    pub fn new() -> JsonLines {
        JsonLines::default()
    }

    /// Any serializable value (structs leave out their `None` fields, as Swift's encoder does).
    pub fn emit<T: Serialize>(&mut self, value: &T) {
        if let Ok(v) = serde_json::to_value(value) {
            self.lines.push(v.to_string());
        }
    }

    /// An object; `Value::Null` fields stay (Swift `NSNull`).
    pub fn emit_value(&mut self, v: Value) {
        self.lines.push(v.to_string());
    }

    /// A line that is already JSON.
    pub fn raw(&mut self, line: String) {
        self.lines.push(line);
    }

    pub fn text(&self) -> String {
        self.lines.iter().map(|l| format!("{l}\n")).collect()
    }
}

/// A stable 64-bit FNV-1a hash, lowercase hex without leading zeros (Swift `String(h, radix: 16)`).
pub fn fnv1a(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sorted_keys_and_slashes() {
        let mut o = JsonLines::new();
        o.emit_value(json!({"type": "log", "path": "/a/b", "agent": "claude", "x": null}));
        assert_eq!(o.lines[0], r#"{"agent":"claude","path":"/a/b","type":"log","x":null}"#);
        assert_eq!(fnv1a(""), "cbf29ce484222325");
        assert_eq!(fnv1a("a"), "af63dc4c8601ec8c");
    }
}
