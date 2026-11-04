//! `JSONSerialization` casts (`as? String`, `as? [[String: Any]]`, NSNumber bridging) over `serde_json::Value`.

use serde_json::{Map, Value};

pub type Obj = Map<String, Value>;

/// `[String: Any]` from a line, or nil (not JSON, or not an object).
pub fn parse_obj(s: &str) -> Option<Obj> {
    match serde_json::from_str::<Value>(s) {
        Ok(Value::Object(o)) => Some(o),
        _ => None,
    }
}

/// `as? String`
pub fn str_(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

/// `as? [String: Any]`
pub fn obj(v: Option<&Value>) -> Option<&Obj> {
    v.and_then(Value::as_object)
}

/// `as? [[String: Any]]`: nil unless every element is an object.
pub fn objs(v: Option<&Value>) -> Option<Vec<&Obj>> {
    v.and_then(Value::as_array)?.iter().map(Value::as_object).collect()
}

/// `as? [String]`: nil unless every element is a string.
pub fn strs(v: Option<&Value>) -> Option<Vec<&str>> {
    v.and_then(Value::as_array)?.iter().map(Value::as_str).collect()
}

/// `as? [Any]`
pub fn arr(v: Option<&Value>) -> Option<&Vec<Value>> {
    v.and_then(Value::as_array)
}

/// `as? Bool`: a JSON bool, or an NSNumber that is exactly 0 or 1.
pub fn bool_(v: Option<&Value>) -> Option<bool> {
    match v? {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_f64().and_then(|x| match x {
            0.0 => Some(false),
            1.0 => Some(true),
            _ => None,
        }),
        _ => None,
    }
}

/// `as? Int`: an NSNumber that is a whole number (a bool is 0 or 1).
pub fn int(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Bool(b) => Some(*b as i64),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                return Some(i);
            }
            let x = n.as_f64()?;
            (x.fract() == 0.0 && x >= i64::MIN as f64 && x < i64::MAX as f64).then_some(x as i64)
        }
        _ => None,
    }
}

/// `as? Double`
pub fn f64_(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

/// `is NSNumber` (bools are NSNumbers too).
pub fn is_number(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Number(_)) | Some(Value::Bool(_)))
}

/// `NSNumber.intValue` (a fraction is cut toward zero).
pub fn int_value(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Bool(b) => Some(*b as i64),
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|x| x as i64)),
        _ => None,
    }
}

/// `JSONSerialization.data(withJSONObject:options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])`.
pub fn pretty(v: &Value) -> String {
    let mut out = String::new();
    write(v, 0, &mut out);
    out
}

/// `pretty` of an object, without copying it into a `Value`.
pub fn pretty_obj(o: &Obj) -> String {
    let mut out = String::new();
    write_obj(o, 0, &mut out);
    out
}

fn indent(n: usize, out: &mut String) {
    for _ in 0..n {
        out.push_str("  ");
    }
}

fn write(v: &Value, depth: usize, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&number(n)),
        Value::String(s) => string(s, out),
        Value::Array(a) => {
            out.push_str("[\n");
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                indent(depth + 1, out);
                write(x, depth + 1, out);
            }
            out.push('\n');
            indent(depth, out);
            out.push(']');
        }
        Value::Object(o) => write_obj(o, depth, out),
    }
}

fn write_obj(o: &Obj, depth: usize, out: &mut String) {
    out.push_str("{\n");
    let mut keys: Vec<&String> = o.keys().collect();
    keys.sort();
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            out.push_str(",\n");
        }
        indent(depth + 1, out);
        string(k, out);
        out.push_str(" : ");
        write(&o[k.as_str()], depth + 1, out);
    }
    out.push('\n');
    indent(depth, out);
    out.push('}');
}

fn number(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    let x = n.as_f64().unwrap_or(0.0);
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

fn string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}
