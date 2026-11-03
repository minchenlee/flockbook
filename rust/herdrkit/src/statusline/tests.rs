//! Ports of ChecksStatusLine (the reading side: ANSI text, info, tags, row fit, segments, the folder).

use super::ansi_text::{lines, plain, BASIC, MAX_COLUMNS, MAX_LINES};
use super::status_segments::{drop_repeats, split};
use super::status_tag_row::{chevron_symbol, fit, Fit};
use super::*;
use std::time::Duration;

const ESC: &str = "\u{1B}";

fn r(t: &str) -> StyledRun {
    StyledRun::new(t)
}

fn texts(segs: &[Vec<StyledRun>]) -> Vec<String> {
    segs.iter().map(|s| s.iter().map(|r| r.text.as_str()).collect()).collect()
}

const INPUT: &str = r#"{"session_id":"5ad83985-b099-4ce9-82e6-85dd73af9b4a","transcript_path":"/x/y.jsonl","model":{"id":"claude-opus-5-5","display_name":"Opus 5.5"},
 "cost":{"total_cost_usd":0.4213,"total_duration_ms":45000},"effort":{"level":"high"},
 "context_window":{"context_window_size":1000000,"used_percentage":37.4,"total_input_tokens":374000}}"#;
const SID: &str = "5ad83985-b099-4ce9-82e6-85dd73af9b4a";

fn info() -> Option<ClaudeStatusInfo> {
    ClaudeStatusInfo::from_json(INPUT.as_bytes())
}

#[test]
fn ansi() {
    let e = ESC;
    assert_eq!(
        lines(&format!("{e}[0m{e}[38;2;136;136;136mabc{e}[0m {e}[1;32mok{e}[0m")),
        vec![vec![r("abc").fg("#888888"), r(" "), r("ok").fg(BASIC[2]).bold()]],
        "truecolor, reset, bold + basic green"
    );
    assert_eq!(lines(&format!("{e}[38;5;196mred{e}[39m plain")), vec![vec![r("red").fg("#ff0000"), r(" plain")]], "256-colour cube, 39 resets");
    assert_eq!(lines(&format!("{e}[38;5;244mg")), vec![vec![r("g").fg("#808080")]], "256-colour grey ramp");
    assert_eq!(lines(&format!("{e}[91mb{e}[38;5;9mc")), vec![vec![r("bc").fg(BASIC[9])]], "bright colours; same style merges");
    assert_eq!(
        lines(&format!("{e}[48;2;1;2;3;2;3;4mx{e}[22;23;24;49my")),
        vec![vec![r("x").bg("#010203").dim().italic().underline(), r("y")]],
        "background, dim, italic, underline and their resets"
    );
    assert_eq!(lines("one\r\ntwo\n"), vec![vec![r("one")], vec![r("two")]], "lines; a last newline adds none");
    assert_eq!(lines(&format!("{e}]8;;https://example.com{e}\\PR #1{e}]8;;{e}\\ done")), vec![vec![r("PR #1 done")]], "an OSC 8 link keeps its text");
    assert_eq!(lines(&format!("a{e}[2Kb{e}[?25lc\u{07}d\u{00}e\tf")), vec![vec![r("abcde f")]], "other escapes and control characters go; a tab is a space");
    assert_eq!(
        lines("a\u{202E}b\u{2066}c\u{2069}\u{200B}d\u{200F}\u{FEFF}e\u{061C}👩\u{200D}💻"),
        vec![vec![r("abcde👩\u{200D}💻")]],
        "direction and zero-width marks go; a joiner in an emoji stays"
    );
    assert_eq!(lines(&format!("{e}[1mstill bold\nnext")), vec![vec![r("still bold").bold()], vec![r("next").bold()]], "a style goes on over a newline");
    assert!(
        lines(&format!("x{e}[")) == vec![vec![r("x")]] && lines(&format!("{e}[38;5;999my{e}[38;2;300mz")) == vec![vec![r("yz")]],
        "cut or out-of-range sequences are dropped"
    );
    assert_eq!(lines(&"a\n".repeat(20)).len(), MAX_LINES, "at most 8 lines");
    let long = lines(&"é".repeat(5000));
    assert_eq!(long[0].iter().map(|r| r.text.graphemes(true).count()).sum::<usize>(), MAX_COLUMNS, "a line is cut at 400 characters");
    // Graphemes, not scalars: a decomposed é is one character.
    let decomposed = lines(&"e\u{301}".repeat(500));
    assert_eq!(decomposed[0][0].text.chars().count(), 800);
    assert!(lines("").is_empty() && lines("\n\n").is_empty(), "nothing to show: no lines");
    assert_eq!(plain(&[vec![r("a").bold(), r("b")], vec![r("c")]]), "ab\nc", "plain text");
}

#[test]
fn info_and_tags() {
    let i = info().unwrap();
    assert!(
        i.session == SID
            && i.model.as_deref() == Some("Opus 5.5")
            && i.effort.as_deref() == Some("high")
            && i.cost_usd == Some(0.4213)
            && i.context_percent == Some(37)
            && i.context_size == Some(1_000_000),
        "session, model, effort, cost, context ({i:?})"
    );
    assert_eq!(i.summary(), "Opus 5.5 · high · ctx 37% · $0.42", "the summary line");
    let bare = ClaudeStatusInfo::from_json(br#"{"session_id":"abc","model":{"display_name":"Haiku"}}"#).unwrap();
    assert!(bare.cost_usd.is_none() && bare.context_percent.is_none() && bare.summary() == "Haiku", "missing fields stay out");
    for bad in [&br#"{"session_id":"../../etc"}"#[..], br#"{"model":{}}"#, b"not json", br#"{"session_id":"a/b"}"#] {
        assert!(ClaudeStatusInfo::from_json(bad).is_none(), "no usable session id: None");
    }
    // display_name empty: no model (not the id); display_name missing: the id.
    assert_eq!(ClaudeStatusInfo::from_json(br#"{"session_id":"a","model":{"display_name":"","id":"x"}}"#).unwrap().model, None);
    assert_eq!(ClaudeStatusInfo::from_json(br#"{"session_id":"a","model":{"id":"x"}}"#).unwrap().model.as_deref(), Some("x"));

    let tags = i.tags();
    assert!(
        tags.iter().map(|t| t.kind).collect::<Vec<_>>() == [StatusTagKind::Model, StatusTagKind::Effort, StatusTagKind::Context, StatusTagKind::Cost]
            && tags.iter().map(|t| t.text.as_str()).collect::<Vec<_>>() == ["Opus 5.5", "high", "ctx 37%", "$0.42 est."]
            && tags[2].percent == Some(37)
            && !tags[2].warn,
        "model, effort, context, cost"
    );
    assert_eq!(
        tags.iter().map(|t| t.spoken.as_str()).collect::<Vec<_>>(),
        ["Model Opus 5.5", "Effort high", "Context 37% used", "Session cost about $0.42, an estimate"],
        "spoken labels"
    );
    assert_eq!(tags.iter().map(|t| t.kind.raw()).collect::<Vec<_>>(), ["model", "effort", "context", "cost"]);
    let hot = ClaudeStatusInfo::from_json(br#"{"session_id":"a","context_window":{"used_percentage":80}}"#).unwrap().tags();
    let cool = ClaudeStatusInfo::from_json(br#"{"session_id":"a","context_window":{"used_percentage":79.4}}"#).unwrap().tags();
    assert!(hot.len() == 1 && hot[0].kind == StatusTagKind::Context && hot[0].warn && !cool[0].warn, "context from 80% is a warning");
    assert!(ClaudeStatusInfo::from_json(br#"{"session_id":"a"}"#).unwrap().tags().is_empty(), "nothing sent, no tags");
    for dark in [false, true] {
        assert!(StatusTag::contrast(StatusTag::warn_hex(dark), StatusTag::tag_hex(dark)) >= 4.5, "the orange reads");
    }
    assert!(
        ClaudeStatusInfo::from_json(br#"{"session_id":"a","context_window":{"used_percentage":1e300}}"#).unwrap().context_percent == Some(100)
            && ClaudeStatusInfo::from_json(br#"{"session_id":"a","context_window":{"used_percentage":-4}}"#).unwrap().context_percent == Some(0),
        "a context percent out of range is clamped"
    );
}

#[test]
fn row_fit() {
    let w = [60.0, 40.0, 70.0, 60.0];
    let f = |a: f64, more: bool| fit(&w, 20.0, 6.0, a, more);
    assert_eq!(f(400.0, false), Fit { shown: 4, chevron: false }, "all fit, nothing more: no chevron");
    assert_eq!(f(248.0, false), Fit { shown: 4, chevron: false }, "exactly fits");
    assert_eq!(f(400.0, true), Fit { shown: 4, chevron: true }, "all fit, own lines to open: the chevron");
    assert_eq!(f(247.0, false), Fit { shown: 3, chevron: true }, "one left out: three and the chevron");
    assert_eq!(f(30.0, false), Fit { shown: 0, chevron: true }, "none fits: the chevron alone");
    assert_eq!(fit(&[], 20.0, 6.0, 400.0, true), Fit { shown: 0, chevron: true }, "no tags, own lines: the chevron");
    assert!(chevron_symbol(false) == "chevron.up" && chevron_symbol(true) == "chevron.down");
}

#[test]
fn segments_and_record_tags() {
    let e = ESC;
    assert_eq!(texts(&split(&[vec![r("a · b | c│d • e   f  ·  ")]])), ["a", "b", "c", "d", "e", "f"], "the separators");
    assert_eq!(texts(&split(&[vec![r("  just one line ")]])), ["just one line"], "no separator: the line, trimmed");
    assert!(split(&[vec![r("   ")], vec![]]).is_empty(), "blank lines: none");
    let ls = lines(&format!(
        "{e}[38;5;141mOpus 5.5{e}[0m {e}[2m·{e}[0m {e}[36m⎇ main{e}[0m {e}[2m·{e}[0m ctx {e}[33m37%{e}[0m {e}[2m·{e}[0m session {e}[32m$0.42{e}[0m {e}[2m·{e}[0m day {e}[1m$11.40{e}[0m/$25\n{e}[2mrate limit 5h: 23% · 7d: 61%{e}[0m"
    ));
    let segs = split(&ls);
    assert_eq!(texts(&segs), ["Opus 5.5", "⎇ main", "ctx 37%", "session $0.42", "day $11.40/$25", "rate limit 5h: 23%", "7d: 61%"], "a real line");
    assert!(
        segs.len() == 7
            && segs[1] == vec![r("⎇ main").fg(BASIC[6])]
            && segs[2] == vec![r("ctx "), r("37%").fg(BASIC[3])]
            && segs[4] == vec![r("day "), r("$11.40").bold(), r("/$25")]
            && segs[5].iter().all(|r| r.dim),
        "each part keeps its colours"
    );
    let rec = StatusLineRecord { lines: ls.clone(), info: info(), time: Utc::now() };
    let tags = rec.tags();
    assert!(
        tags.iter().map(|t| t.text.as_str()).collect::<Vec<_>>()
            == ["Opus 5.5", "high", "ctx 37%", "$0.42 est.", "⎇ main", "day $11.40/$25", "rate limit 5h: 23%", "7d: 61%"]
            && tags[4].kind == StatusTagKind::Own
            && tags[4].runs == vec![r("⎇ main").fg(BASIC[6])],
        "JSON first, then the owner's parts without the repeats"
    );
    let near = split(&[vec![r("Opus 5.5 (1M) · ctx 38% · $0.42 · session: $0.42 · context 37%")]]);
    assert_eq!(texts(&drop_repeats(&near, info().as_ref())), ["Opus 5.5 (1M)", "ctx 38%", "$0.42"], "only exact repeats go");
    assert_eq!(StatusLineRecord { lines: ls, info: None, time: Utc::now() }.tags()[0].text, "Opus 5.5", "no facts: the owner's parts");
    for dark in [false, true] {
        let c = StatusTag::contrast(StatusTag::dim_hex(dark), StatusTag::tag_hex(dark));
        assert!(c >= 4.5 && StatusTag::dim_hex(dark) == if dark { "#969696" } else { "#6b6b6b" });
        assert!(
            StatusTag::run_hex(&r("x").dim(), dark).as_deref() == Some(StatusTag::dim_hex(dark))
                && StatusTag::run_hex(&r("x").fg("#cd0000").dim(), dark).as_deref() == Some(StatusTag::dim_hex(dark))
                && StatusTag::run_hex(&r("x").fg("#000000"), false).as_deref() == Some("#000000")
                && StatusTag::run_hex(&r("x"), dark).is_none()
        );
    }
    assert!(!StatusTag::readable("#808080", false) && !StatusTag::readable("#6e6e6e", true) && StatusTag::readable("#cd0000", false));
    assert!(
        !StatusTag::readable("#ffff00", false) && StatusTag::readable("#ffff00", true) && StatusTag::readable("#000000", false) && !StatusTag::readable("#000000", true)
    );
}

#[test]
fn folder_record() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("statusline");
    let f = StatusLineFolder::new(&dir);
    let now = SystemTime::now;
    assert!(f.record(SID, now()).is_none(), "nothing saved, no record");
    f.save(INPUT.as_bytes(), format!("{ESC}[32mmain{ESC}[0m $0.42").as_bytes(), SID);
    let rec = f.record(SID, now()).unwrap();
    assert!(rec.lines == vec![vec![r("main").fg(BASIC[2]), r(" $0.42")]] && rec.info.as_ref().unwrap().cost_usd == Some(0.4213), "the saved line and info come back");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join(format!("{SID}.out"))).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "files are mode 600");
        let dmode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dmode, 0o700, "the folder is mode 700");
    }
    f.save(INPUT.as_bytes(), b"", SID);
    let rec = f.record(SID, now()).unwrap();
    assert!(rec.lines.is_empty() && !rec.is_empty(), "no own line: still shown (the info's tags)");
    assert!(
        StatusLineRecord { lines: vec![], info: ClaudeStatusInfo::from_json(br#"{"session_id":"a"}"#), time: Utc::now() }.is_empty(),
        "no own line and no facts: nothing to show"
    );
    assert!(f.record(&format!("../{SID}"), now()).is_none() && f.record("", now()).is_none(), "a session id that is not a plain name is not read");
    // A record older than a day is not shown; clean removes it.
    let old = now() - Duration::from_secs_f64(2.0 * StatusLineFolder::MAX_AGE);
    for ext in ["out", "json"] {
        std::fs::File::options().write(true).open(dir.join(format!("{SID}.{ext}"))).unwrap().set_modified(old).unwrap();
    }
    assert!(f.record(SID, now()).is_none(), "a record older than a day is not shown");
    f.clean(now());
    assert!(!dir.join(format!("{SID}.out")).exists(), "clean removes old records");
    // A link is not followed; an output over the limit is not read.
    f.save(INPUT.as_bytes(), &vec![b'x'; 70_000], SID);
    let rec = f.record(SID, now()).unwrap();
    assert_eq!(rec.lines[0][0].text.len(), 400, "saved output is cut at 64 KB, a line at 400 characters");
    #[cfg(unix)]
    {
        std::fs::write(dir.join("real.json"), INPUT).unwrap();
        std::os::unix::fs::symlink(dir.join("real.json"), dir.join("linked.json")).unwrap();
        assert!(f.record("linked", now()).is_none(), "a linked record is not read");
    }
}
