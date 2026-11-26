//! Swift: `HCKeys`. Key names for hc's `terminal.keys` ("enter, esc, tab, up, ctrl+c, alt+x, f5, a, ...").

/// herdr / tmux spelling → hc spelling. Unknown names pass through lowercased.
pub fn translate(k: &str) -> String {
    if k.chars().count() == 1 {
        return k.to_string(); // a literal character keeps its case
    }
    let l = k.to_lowercase();
    let mapped = match l.as_str() {
        "escape" => Some("esc"),
        "return" => Some("enter"),
        "bspace" => Some("backspace"),
        "btab" => Some("shift+tab"),
        "pageup" => Some("pageup"),
        "pagedown" => Some("pagedown"),
        "ppage" => Some("pageup"),
        "npage" => Some("pagedown"),
        "dc" => Some("delete"),
        _ => None,
    };
    if let Some(m) = mapped {
        return m.to_string();
    }
    for (p, r) in [("c-", "ctrl+"), ("m-", "alt+"), ("s-", "shift+")] {
        if l.starts_with(p) {
            let rest: String = k.chars().skip(2).collect();
            return format!("{r}{}", translate(&rest));
        }
    }
    l
}

/// The characters hc's `terminal.keys` can type (tested live 2026-11-11): its key names are read without case
/// ("A" types "a"), so a capital goes as "shift+a"; digits, space and the unshifted symbols type as themselves;
/// every other character (! ? ( ) " : and anything outside ASCII) is refused as an unknown key name, and "shift+1"
/// types "1", not "!". Typed answers are checked with this before any key goes (`PaneScreen::can_type`).
pub fn typeable(c: char) -> bool {
    c.is_ascii_alphanumeric() || " ',.;/-=[]\\`".contains(c)
}

/// Bytes → key names: CR/LF enter, ESC esc, tab, DEL backspace, 0x01–0x1A ctrl+letter, arrow escape sequences,
/// a capital letter as shift+letter, every other character as itself.
pub fn names(bytes: &[u8]) -> Vec<String> {
    let s = String::from_utf8_lossy(bytes);
    let chars: Vec<char> = s.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\u{1b}' && chars.len() - i >= 3 && chars[i + 1] == '[' {
            let arrow = match chars[i + 2] {
                'A' => Some("up"),
                'B' => Some("down"),
                'C' => Some("right"),
                'D' => Some("left"),
                'H' => Some("home"),
                'F' => Some("end"),
                'Z' => Some("shift+tab"),
                _ => None,
            };
            if let Some(a) = arrow {
                out.push(a.to_string());
                i += 3;
                continue;
            }
        }
        i += 1;
        let v = c as u32;
        out.push(match v {
            0x0D | 0x0A => "enter".into(),
            0x1B => "esc".into(),
            0x09 => "tab".into(),
            0x7F | 0x08 => "backspace".into(),
            0x01..=0x1A => format!("ctrl+{}", char::from_u32(v + 0x60).unwrap()),
            0x20 => "space".into(),
            _ if c.is_ascii_uppercase() => format!("shift+{}", c.to_ascii_lowercase()),
            _ => c.to_string(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_names() {
        assert_eq!(translate("Enter"), "enter");
        assert_eq!(translate("Escape"), "esc");
        assert_eq!(translate("Down"), "down");
        assert_eq!(translate("C-c"), "ctrl+c");
        assert_eq!(translate("M-x"), "alt+x");
        assert_eq!(translate("BTab"), "shift+tab");
        assert_eq!(translate("A"), "A");
        assert_eq!(translate("1"), "1");
    }

    #[test]
    fn bytes_to_names() {
        assert_eq!(names(b"a b\r"), vec!["a", "space", "b", "enter"]);
        assert_eq!(names(b"\x1b[A\x1b[B\x1b"), vec!["up", "down", "esc"]);
        assert_eq!(names(b"\x03\t\x7f"), vec!["ctrl+c", "tab", "backspace"]);
        assert_eq!(names(b"Ol 1."), vec!["shift+o", "l", "space", "1", "."], "a capital goes as shift+letter (hc reads names without case)");
    }

    #[test]
    fn typeable_chars() {
        assert!("Olives please, 2 of them; a/b-c=d [x] \\ `q` it's".chars().all(typeable));
        for c in ['!', '?', '(', ')', '"', ':', 'é', '日', '\n', '\t'] {
            assert!(!typeable(c), "not typeable: {c:?}");
        }
    }
}
