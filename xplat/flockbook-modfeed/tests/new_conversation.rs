//! A send after `/clear` (issue #11, item 3): the same agent process now writes a new log. The send says "new
//! conversation", not "another agent", and the locate cache drops the old log at once (not after RECHECK_S), so the
//! chat's next `feed` reads the new one. Another agent in the pane is still refused as before.
#![cfg(unix)]

mod common;
use common::{terminal, FakeHc};
use herdr_core::hc::{HcClient, HcSession};
use herdr_core::*;
use herdr_core::resolver::SessionResolver;
use modfeedkit::{cards, locate};
use std::time::SystemTime;

#[test]
fn send_after_clear_is_a_new_conversation() {
    let fake = FakeHc::start();
    let hc = HcClient::new(HcSession::new(&fake.path), 10, 60);
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let (old_log, new_log) = (dir.join("old.jsonl"), dir.join("new.jsonl"));
    std::fs::write(&old_log, "{}\n").unwrap();
    std::fs::write(&new_log, "{}\n").unwrap();
    let old_s = old_log.to_string_lossy().to_string();
    fake.set_terminals(vec![terminal("t1", &["claude"], Some(4242), Some("idle"), None)]);
    let ws = hc.snapshot().unwrap();
    let pane = locate::pane(&ws, "t1").unwrap().clone();
    let now_s = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs_f64();

    // The chat's feed cached the old log a moment ago, for this agent process.
    let cache = dir.join("cache.json");
    let mut c = locate::Cache::default();
    c.entries.insert("t1".into(), locate::Entry { kind: "claude".into(), cwd: pane.cwd.clone(), pids: pane.process_ids.clone(), process: pane.agent.as_ref().and_then(|a| a.process.clone()), path: Some(old_s.clone()), checked: now_s });
    locate::write_cache(&c, Some(&cache));

    // The send looks the log up again (the cache is fresh, but a send never trusts it) and gives back the old entry.
    let (found, was) = locate::recheck("t1", &ws, &mut SessionResolver::new("/nonexistent.sock"), Some(&cache), now_s + 1.0).unwrap();
    assert!(!found.cached, "a send looks the log up again");
    assert_eq!(was.as_ref().and_then(|e| e.path.as_deref()), Some(old_s.as_str()), "the entry the feed had");
    let after = locate::read_cache(Some(&cache));
    let e = after.entries.get("t1").expect("the entry is written again");
    assert!(e.path.as_deref() != Some(old_s.as_str()) && e.checked == now_s + 1.0, "the feed no longer reads the old log ({e:?})");

    // Same agent process, another log: a new conversation. Nothing is sent; the message says so.
    let sends = || fake.calls("terminal.send").len();
    let s0 = sends();
    let d = cards::send(&pane, &hc, "hi", &[], Some(&new_log), Some("claude"), Some(&old_s), was.as_ref());
    let m = d.message.clone().unwrap_or_default();
    assert!(d.result == "changed" && m.contains("new conversation") && !m.contains("another agent"), "same process: new conversation ({d:?})");
    assert_eq!(sends(), s0, "nothing sent");

    // Another process in the pane: another agent, as before.
    let mut other = was.clone().unwrap();
    other.pids = vec![9999];
    let d = cards::send(&pane, &hc, "hi", &[], Some(&new_log), Some("claude"), Some(&old_s), Some(&other));
    assert!(d.result == "changed" && d.message.as_deref().unwrap_or("").contains("another agent"), "another process ({d:?})");

    // The cached entry was for another log than the one the chat showed: no proof, refused as before.
    let mut elsewhere = was.clone().unwrap();
    elsewhere.path = Some(dir.join("third.jsonl").to_string_lossy().into());
    let d = cards::send(&pane, &hc, "hi", &[], Some(&new_log), Some("claude"), Some(&old_s), Some(&elsewhere));
    assert!(d.result == "changed" && d.message.as_deref().unwrap_or("").contains("another agent"), "no proof ({d:?})");

    // No log found now (the lookup failed): the chat could not show a new one, so refused as before.
    let d = cards::send(&pane, &hc, "hi", &[], None, Some("claude"), Some(&old_s), was.as_ref());
    assert!(d.message.as_deref().unwrap_or("").contains("another agent"), "no log now ({d:?})");

    // No entry, or another agent kind: refused as before.
    let d = cards::send(&pane, &hc, "hi", &[], Some(&new_log), Some("claude"), Some(&old_s), None);
    assert!(d.message.as_deref().unwrap_or("").contains("another agent"), "no entry ({d:?})");
    let d = cards::send(&pane, &hc, "hi", &[], Some(&new_log), Some("codex"), Some(&old_s), was.as_ref());
    assert!(d.message.as_deref().unwrap_or("").contains("another agent"), "another kind ({d:?})");
    assert_eq!(sends(), s0, "nothing sent in any case");
}
