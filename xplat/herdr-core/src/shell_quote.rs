//! POSIX shell words. The caller must select a POSIX shell before using these on Windows.
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metacharacters_and_unicode() {
        for s in [
            "$HOME", "`id`", "a\nb", "a'b", "'\u{301}", "資料", "", "a\\b",
        ] {
            assert_eq!(quoted_if_needed(s), quoted(s));
        }
        assert_eq!(quoted("'\u{301}"), "''\\''\u{301}'");
        assert_eq!(quoted_if_needed("/tmp/a-b.png"), "/tmp/a-b.png");
    }
}
pub fn quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
pub fn quoted_if_needed(s: &str) -> String {
    if !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-+=:@%,".contains(&b))
    {
        s.into()
    } else {
        quoted(s)
    }
}
