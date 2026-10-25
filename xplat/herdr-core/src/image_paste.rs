//! Explicit composer image attachments, never inferred from arbitrary message text.
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_markers_do_not_acknowledge_input() {
        assert_eq!(input_markers("[Image #1]\n❯ waiting", "claude"), 0);
        assert_eq!(input_markers("› [Image #1] [Image #2]", "codex"), 2);
        assert_eq!(input_markers("│ ❯ [Image #1]", "grok"), 1);
        assert_eq!(input_markers("history [Image #1]", "claude"), 0);
    }
    #[test]
    fn reject_control_paths() {
        assert!(paste("/tmp/x\u{1b}[201~").is_err());
    }
}
pub fn waits_for_marker(kind: &str) -> bool {
    matches!(kind, "claude" | "codex" | "grok")
}
pub fn paste(path: &str) -> Result<String, String> {
    if path.chars().any(char::is_control) {
        return Err("Image path contains a control character".into());
    }
    Ok(format!("\u{1b}[200~{path}\u{1b}[201~"))
}
pub fn input_markers(screen: &str, kind: &str) -> usize {
    let lines: Vec<_> = screen.lines().collect();
    let start = lines.iter().rposition(|l| {
        let t = l.trim_start_matches(' ');
        match kind {
            "codex" => t.starts_with('›'),
            "grok" => t
                .strip_prefix('│')
                .is_some_and(|s| s.trim_start_matches(' ').starts_with('❯')),
            _ => t.starts_with('❯'),
        }
    });
    start
        .map(|i| lines[i..].join("\n").matches("[Image #").count())
        .unwrap_or(0)
}
