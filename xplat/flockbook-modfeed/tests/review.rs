//! An approval binds to the box on screen, never to a lagging log's call; one
//! answer per prompt; continued logs; paste cleanup; no identity; send bound to the agent shown.
#![cfg(unix)]

mod common;
use common::{claude, terminal, FakeHc};
use herdr_core::hc::{HcClient, HcSession};
use herdr_core::*;
use herdr_core::resolver::SessionResolver;
use modfeedkit::{answered, cards, locate, paste};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

fn tool(id: &str, call: &str, cmd: &str) -> String {
    format!(r#"{{"type":"assistant","uuid":"{id}","timestamp":"2026-11-03T01:40:00.000Z","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"{call}","name":"Bash","input":{{"command":"{cmd}"}}}}]}}}}"#)
}
fn claude_box(cmd: &str) -> String {
    format!("✻ Working… 3s\n────────────────────\n Bash command\n Run it\n╌╌╌╌╌╌╌╌╌╌\n {cmd}\n╌╌╌╌╌╌╌╌╌╌\n Do you want to proceed?\n ❯ 1. Yes\n   2. No\n\n Esc to cancel")
}
fn grok(command: &str, focus: usize) -> String {
    let rows = ["Yes, and don't ask again for anything (always-approve mode)", "Yes, proceed", "No, reject (type to add feedback)", "Never allow: echo"];
    let mut l: Vec<String> = ["  ❯ Write a file.", "", "  ◆ Working… 3.0s", "", "  ┃", "  ┃  ›  Run a command"].iter().map(|s| s.to_string()).collect();
    l.push(format!("  ┃  │ {command}"));
    l.push("  ┃  ← → narrow scope".into());
    l.push("  ┃".into());
    for (i, r) in rows.iter().enumerate() {
        l.push(format!("  ┃  {} ({}) {r}", i + 1, if i + 1 == focus { "●" } else { "○" }));
    }
    l.push("  ┃".into());
    l.push(String::new());
    l.push("  1/4:select  │  Tab:next option  │  ←/→:scope  │  Ctrl+o:always-approve  │  Ctrl+c:cancel  │  Esc:scrollback".into());
    l.join("\n")
}
fn write_log(dir: &Path, name: &str, lines: &[String]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, lines.join("\n") + "\n").unwrap();
    p
}
fn pane_of(hc: &HcClient, t: &str) -> Pane {
    locate::pane(&hc.snapshot().unwrap(), t).unwrap().clone()
}

#[test]
fn review1() {
    let fake = FakeHc::start();
    let hc = HcClient::new(HcSession::new(&fake.path), 10, 60);
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // 1. The log still names call A; the screen already shows B's box.
    fake.set_terminals(vec![claude("t1", "blocked", "permission")]);
    let pane = pane_of(&hc, "t1");
    let lagging = write_log(dir, "a.jsonl", &[tool("a1", "toolu_A", "ls -la")]);
    fake.set_screen("t1", &claude_box("rm -rf build"));
    let card = cards::card(&pane, &hc, Some(&lagging), false, None);
    assert!(
        card.kind.as_deref() == Some("approval") && card.request.as_deref() != Some("toolu_A") && card.command.as_deref().unwrap().contains("rm -rf build"),
        "#1: the card is B, not A's id ({card:?})"
    );
    let k0 = fake.keys().len();
    let stale = cards::approve(&pane, &hc, Some(&lagging), "toolu_A", true, None);
    assert!(stale.result == "changed" && fake.keys().len() == k0, "#1: approve with A's id while B is on screen: nothing sent");
    let caught = write_log(dir, "b.jsonl", &[tool("a2", "toolu_B", "rm -rf build")]);
    let cb = cards::card(&pane, &hc, Some(&caught), false, None);
    assert_eq!(cb.request.as_deref(), Some("toolu_B"), "#1: log and box are the same call: the log's id");

    let r1 = cards::card(&pane, &hc, None, false, None).request;
    fake.set_screen("t1", &claude_box("ls"));
    let r2 = cards::card(&pane, &hc, None, false, None).request;
    assert!(r1.is_some() && r2.is_some() && r1 != r2, "#1: two commands, two screen ids");
    let k1 = fake.keys().len();
    let next = cards::approve(&pane, &hc, None, r1.as_deref().unwrap(), true, None);
    assert!(next.result == "changed" && fake.keys().len() == k1, "#1: the id of the box before does not answer this box");

    // Grok.
    fake.set_terminals(vec![terminal("t2", &["grok"], Some(4242), Some("blocked"), Some("permission"))]);
    let gpane = pane_of(&hc, "t2");
    fake.set_screen("t2", &grok("echo one", 2));
    let g1 = cards::card(&gpane, &hc, None, false, None).request;
    fake.set_screen("t2", &grok("echo two", 2));
    let g2 = cards::card(&gpane, &hc, None, false, None).request;
    assert!(g1.is_some() && g2.is_some() && g1 != g2, "#1: grok, two commands, two ids");
    let k2 = fake.keys().len();
    fake.on_keys(Some(Arc::new(|a, k, st| {
        if a == "t2" && (k == ["enter"] || k == ["Enter"]) {
            st.screens.insert("t2".into(), "  ❯ ".into());
        }
    })));
    let ga = cards::approve(&gpane, &hc, None, g2.as_deref().unwrap(), true, None);
    fake.on_keys(None);
    let sent = fake.keys()[k2..].to_vec();
    assert!(ga.result == "sent" && sent.len() == 1 && sent[0][0].to_lowercase() == "enter", "#1: grok allow is Enter on Yes, proceed ({ga:?}, {sent:?})");

    // 2. One approval goes out once.
    let cache = dir.join("cache.json");
    let c = Some(cache.as_path());
    fake.set_terminals(vec![claude("t1", "blocked", "permission")]);
    fake.set_screen("t1", &claude_box("rm -rf build"));
    let id2 = cards::card(&pane, &hc, None, false, c).request.unwrap();
    let slow = HcClient::new(HcSession::new(&fake.path), 1, 2);
    fake.keys_delay(2.0);
    let k3 = fake.keys().len();
    let held = cards::approve(&pane, &slow, None, &id2, true, c);
    fake.keys_delay(0.0);
    assert_eq!(held.result, "unconfirmed", "#2: a held key is unconfirmed, not failed ({held:?})");
    std::thread::sleep(Duration::from_millis(1200)); // the held key lands in the fake
    let again = cards::approve(&pane, &hc, None, &id2, true, c);
    assert!(again.result == "answered" && fake.keys().len() == k3 + 1, "#2: a second press sends nothing ({again:?})");
    let shown = cards::card(&pane, &hc, None, false, c);
    assert!(shown.request.as_deref() == Some(id2.as_str()) && shown.answered == Some(true), "#2: the card says answered ({shown:?})");
    fake.set_screen("t1", &claude_box("ls"));
    let _ = cards::card(&pane, &hc, None, false, c);
    fake.set_screen("t1", &claude_box("rm -rf build"));
    let k4 = fake.keys().len();
    let later = cards::approve(&pane, &hc, None, &id2, true, c);
    assert!(later.result == "sent" && fake.keys().len() == k4 + 1, "#2: after the box went away, answered again ({later:?})");
    let lag = cards::approve(&pane, &hc, None, &id2, true, c);
    assert!(lag.result == "answered" && fake.keys().len() == k4 + 1, "#2: box still drawn: the next press sends nothing");

    // Re-check: a failed read or a re-wrapped box keeps the claim; a held key's claim lasts while blocked.
    fake.set_screen("t1", &claude_box("ls"));
    let _ = cards::card(&pane, &hc, None, false, c);
    fake.set_screen("t1", &claude_box("cat notes"));
    let id5 = cards::card(&pane, &hc, None, false, c).request.unwrap();
    fake.keys_delay(2.0);
    let held5 = cards::approve(&pane, &slow, None, &id5, true, c);
    fake.keys_delay(0.0);
    assert_eq!(held5.result, "unconfirmed", "re-check: setup, a held key");
    std::thread::sleep(Duration::from_millis(1200));
    let kr = fake.keys().len();
    fake.clear_screen("t1");
    let _ = cards::card(&pane, &hc, None, false, c);
    fake.set_screen("t1", &claude_box("cat notes"));
    let after_fail = cards::approve(&pane, &hc, None, &id5, true, c);
    assert!(after_fail.result == "answered" && fake.keys().len() == kr, "re-check: a failed screen read keeps the claim ({after_fail:?})");
    let wrapped = claude_box("cat notes").replace(" Run it\n", " Run\n it\n");
    fake.set_screen("t1", &wrapped);
    let wcard = cards::card(&pane, &hc, None, false, c);
    assert!(wcard.request.is_some() && wcard.request.as_deref() != Some(id5.as_str()), "re-check: a re-wrapped box has another screen id");
    assert_eq!(wcard.answered, Some(true), "re-check: the same prompt re-wrapped still shows as answered");
    let wa = cards::approve(&pane, &hc, None, wcard.request.as_deref().unwrap(), true, c);
    assert!(wa.result == "answered" && fake.keys().len() == kr, "re-check: the re-wrapped box sends no second key");
    let old = SystemTime::now() - (answered::HELD_WAIT + Duration::from_secs(5));
    for t in std::fs::read_dir(dir.join("answered")).unwrap().flatten() {
        for u in std::fs::read_dir(t.path()).unwrap().flatten() {
            std::fs::File::options().write(true).open(u.path()).unwrap().set_modified(old).unwrap();
        }
    }
    fake.set_screen("t1", &claude_box("cat notes"));
    let renewed = cards::approve(&pane, &hc, None, &id5, true, c);
    assert!(renewed.result == "answered" && fake.keys().len() == kr, "re-check: a held key's claim lasts while blocked ({renewed:?})");
    fake.set_screen("t1", "  ❯ ");
    let _ = cards::card(&pane, &hc, None, false, c);
    fake.set_screen("t1", &claude_box("cat notes"));
    let fresh = cards::approve(&pane, &hc, None, &id5, true, c);
    assert!(fresh.result == "sent" && fake.keys().len() == kr + 1, "re-check: after a read with no box, answered again ({fresh:?})");
    // Re-review 2: another run is claiming in this folder now: "busy", not "answered", and no key; then it works.
    fake.set_screen("t1", &claude_box("cat busy"));
    let bid = cards::card(&pane, &hc, None, false, c).request.unwrap();
    let lock = std::fs::File::options().read(true).write(true).create(true).truncate(false)
        .open(dir.join("answered").join(modfeedkit::protocol::fnv1a(&pane.id)).join(".lock")).unwrap();
    lock.lock().unwrap();
    let kb = fake.keys().len();
    let busy = cards::approve(&pane, &hc, None, &bid, true, c);
    assert!(busy.result == "busy" && busy.message.is_some() && fake.keys().len() == kb, "re-review 2: busy, no key ({busy:?})");
    drop(lock);
    let after = cards::approve(&pane, &hc, None, &bid, true, c);
    assert!(after.result == "sent" && fake.keys().len() == kb + 1, "re-review 2: the next press sends ({after:?})");

    // 3. A Claude conversation went on in a new log: a cache hit follows continued-in.
    let proj = dir.join("projects/-w-proj");
    std::fs::create_dir_all(&proj).unwrap();
    let (old_log, new_log) = (proj.join("old1.jsonl"), proj.join("new1.jsonl"));
    let lead = r#"{"type":"user","uuid":"l1","sessionId":"s","message":{"role":"user","content":"start"}}"#;
    std::fs::write(&old_log, format!("{lead}\n{}\n", r#"{"type":"continued-in","timestamp":"2026-11-03T06:54:55.989Z","sessionId":"old1","continuedInSessionId":"new1"}"#)).unwrap();
    std::fs::write(&new_log, format!("{lead}\n")).unwrap();
    fake.set_terminals(vec![terminal("t3", &["claude"], Some(4242), Some("working"), None)]);
    let ws3 = hc.snapshot().unwrap();
    let p3 = locate::pane(&ws3, "t3").unwrap().clone();
    let cache3 = dir.join("locate/cache.json");
    let mut c3 = locate::Cache::default();
    let now_s = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs_f64();
    c3.entries.insert(p3.terminal_id.clone(), locate::Entry { kind: "claude".into(), cwd: p3.cwd.clone(), pids: p3.process_ids.clone(), path: Some(old_log.to_string_lossy().into()), checked: now_s });
    locate::write_cache(&c3, Some(&cache3));
    let hit = locate::find("t3", &ws3, &mut SessionResolver::new("/nonexistent.sock"), Some(&cache3), now_s).ok().unwrap();
    assert!(hit.cached && hit.path.as_deref() == Some(new_log.to_string_lossy().as_ref()), "#3: a cache hit follows continued-in ({:?})", hit.path);

    // 4. paste-image cleans only its own old pastes.
    let pastes = dir.join("paste");
    std::fs::create_dir_all(&pastes).unwrap();
    let three_days = SystemTime::now() - Duration::from_secs(3 * 86400);
    for n in ["paste-20261101-101010-ab12cd.png", "holiday.png", "paste-notes.png"] {
        std::fs::write(pastes.join(n), [1]).unwrap();
        std::fs::File::options().write(true).open(pastes.join(n)).unwrap().set_modified(three_days).unwrap();
    }
    paste::save(&[2], &pastes, SystemTime::now()).unwrap();
    let left: Vec<String> = std::fs::read_dir(&pastes).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert!(
        !left.contains(&"paste-20261101-101010-ab12cd.png".to_string()) && left.contains(&"holiday.png".to_string()) && left.contains(&"paste-notes.png".to_string()),
        "#4: an old paste goes; other PNGs stay ({left:?})"
    );

    // 5. An agent Flockbook cannot tell apart: nothing sent, and the reason says so.
    fake.set_terminals(vec![terminal("t4", &["claude"], None, Some("blocked"), Some("permission"))]);
    let p4 = pane_of(&hc, "t4");
    assert!(p4.agent.is_some() && p4.agent.as_ref().unwrap().process.is_none(), "#5: an agent without a process");
    fake.set_screen("t4", &claude_box("rm -rf build"));
    let sends = || fake.calls("terminal.send").len();
    let (k5, s5) = (fake.keys().len(), sends());
    let r4 = cards::card(&p4, &hc, None, false, None).request.unwrap_or("x".into());
    let na = cards::approve(&p4, &hc, None, &r4, true, None);
    let ns = cards::send(&p4, &hc, "hi", &[], None, None, None, None);
    let nq = cards::answer(&p4, &hc, None, &r4, &[QuestionAnswer { picks: vec![0], other: None }]);
    assert!(fake.keys().len() == k5 && sends() == s5, "#5: no identity, nothing sent");
    for (what, d) in [("approve", &na), ("send", &ns)] {
        let m = d.message.as_deref().unwrap_or("");
        assert!(d.result == "no-identity" && m.contains("cannot tell") && !m.contains("another agent"), "#5: {what} ({d:?})");
    }
    assert!(nq.result == "changed" || nq.result == "no-identity", "#5: answer on an approval is refused");

    // 6. send is bound to the agent the owner was reading.
    fake.set_terminals(vec![terminal("t5", &["claude"], Some(4242), Some("idle"), None)]);
    let p5 = pane_of(&hc, "t5");
    let (shown_log, other_log) = (dir.join("shown.jsonl"), dir.join("other.jsonl"));
    let shown_s = shown_log.to_string_lossy().to_string();
    let s6 = sends();
    let wrong_kind = cards::send(&p5, &hc, "hi", &[], Some(&shown_log), Some("codex"), Some(&shown_s), None);
    let wrong_log = cards::send(&p5, &hc, "hi", &[], Some(&other_log), Some("claude"), Some(&shown_s), None);
    assert!(wrong_kind.result == "changed" && wrong_log.result == "changed" && sends() == s6, "#6: nothing typed");
    let same = cards::send(&p5, &hc, "hi", &[], Some(&shown_log), Some("claude"), Some(&shown_s), None);
    assert!(same.result == "sent" && sends() == s6 + 1, "#6: the agent and log shown: sent ({same:?})");
}
