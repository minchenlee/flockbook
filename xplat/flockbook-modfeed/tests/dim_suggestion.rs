//! Claude Code's prompt suggestion (issue #11, item 2): an empty box can show a dim "/clear" the person did not type.
//! The screen's text cannot tell it from typed "/clear", so `send` takes one look at the first paint through a
//! watch-only attach: all dim with the cursor at the box start is a suggestion, and the text is sent. Typed text, or
//! any doubt (the attach refused, no paint in time), is refused as before: `box_not_empty`, nothing typed.
#![cfg(unix)]

mod common;
use common::{claude, FakeHc};
use serde_json::Value;
use std::process::Command;
use std::time::{Duration, Instant};

fn send(sock: &str) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed"))
        .args(["send", "--terminal", "t1", "--text", "hello"])
        .env_clear()
        .env("HC_SOCKET", sock)
        // No settings there: prompt suggestions are on, Claude Code's default.
        .env("HOME", "/nonexistent-home")
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).find(|l| l["type"] == "done").unwrap()
}

/// Redacted from a Claude Code 2.1.322 first paint (spike 11-12): its dim empty-box text is SGR 2 with the cursor at
/// the box start; typed text has a colour and the cursor after it.
fn paint(row: &str, cursor_col: u32) -> Vec<u8> {
    let rule = "─".repeat(40);
    format!("\x1b[2J\x1b[1;1H● Done.\x1b[3;1H{rule}\x1b[4;1H{row}\x1b[5;1H{rule}\x1b[6;3H~/proj\x1b[4;{cursor_col}H").into_bytes()
}

#[test]
fn a_dim_slash_suggestion_does_not_block_a_send() {
    let fake = FakeHc::start();
    fake.set_terminals(vec![claude("t1", "idle", "idle")]);
    let rule = "─".repeat(40);
    fake.set_screen("t1", &format!("● Done.\n\n{rule}\n❯\u{a0}/clear\n{rule}\n  ~/proj\n"));
    let sends = || fake.calls("terminal.send").len();
    // Only the attach, its CLOSE and the hello; no keys, no resize.
    let watch_only = || {
        let frames = fake.calls("frame");
        assert!(frames.iter().all(|f| f["kind"] == 2), "only CLOSE frames ({frames:?})");
        assert!(fake.calls("terminal.attach").iter().all(|a| a["observe"] == true), "watch only");
        assert!(fake.calls("terminal.keys").is_empty(), "no keys");
    };

    // Typed "/clear": a colour, the cursor after it. Refused.
    fake.set_paint("t1", &paint("❯\u{a0}\x1b[0m\x1b[38;2;177;185;249m/clear\x1b[0m", 9));
    assert_eq!(send(&fake.path)["result"], "box_not_empty", "typed text is never typed over");
    assert_eq!(sends(), 0);
    assert_eq!(fake.calls("terminal.attach").len(), 1, "one look");
    watch_only();

    // The attach refused, or no paint in time: refused as before, soon.
    fake.refuse_attach(true);
    assert_eq!(send(&fake.path)["result"], "box_not_empty", "attach refused");
    fake.refuse_attach(false);
    fake.state.lock().unwrap().paints.clear();
    let t0 = Instant::now();
    assert_eq!(send(&fake.path)["result"], "box_not_empty", "no paint");
    assert!(t0.elapsed() < Duration::from_secs(3), "the look ends at its limit ({:?})", t0.elapsed());
    assert_eq!(sends(), 0);
    watch_only();

    // Dim "/clear", the cursor at the box start: Claude Code's suggestion, the box is empty. Sent.
    fake.set_paint("t1", &paint("❯\u{a0}\x1b[0m\x1b[2m/clear\x1b[0m", 3));
    let d = send(&fake.path);
    assert_ne!(d["result"], "box_not_empty", "a dim suggestion is not text ({d})");
    assert_eq!(sends(), 1, "sent");
    watch_only();

    // Dim, but the screen shows other text than the paint: no proof, refused.
    fake.set_screen("t1", &format!("● Done.\n\n{rule}\n❯\u{a0}/compact\n{rule}\n  ~/proj\n"));
    assert_eq!(send(&fake.path)["result"], "box_not_empty", "the paint must read what the screen does");
    assert_eq!(sends(), 1);
}

/// Another agent came into the pane during the look: the gate runs again before anything is typed.
#[test]
fn an_agent_change_during_the_look_is_refused() {
    let fake = FakeHc::start();
    fake.set_terminals(vec![claude("t1", "idle", "idle")]);
    let rule = "─".repeat(40);
    fake.set_screen("t1", &format!("● Done.\n\n{rule}\n❯\u{a0}/clear\n{rule}\n  ~/proj\n"));
    fake.set_paint("t1", &paint("❯\u{a0}\x1b[0m\x1b[2m/clear\x1b[0m", 3));
    let mut other = claude("t1", "idle", "idle");
    other["pid"] = serde_json::json!(9999);
    fake.state.lock().unwrap().after_attach = Some(vec![other]);
    let d = send(&fake.path);
    assert_eq!(fake.calls("terminal.attach").len(), 1, "it looked");
    assert_eq!(d["result"], "changed", "refused: another agent ({d})");
    assert!(fake.calls("terminal.send").is_empty() && fake.calls("terminal.keys").is_empty(), "nothing typed");
}
