//! Issue #11 item 1: an answer the key gate stopped says why. Another agent: "another agent", as before. The same
//! agent with another screen: its own result, and "nothing was sent" only when no key went.
#![cfg(unix)]

mod common;
use common::{claude, FakeHc};
use herdr_core::hc::{HcClient, HcSession};
use herdr_core::*;
use modfeedkit::{cards, locate};

#[test]
fn answer_stop_names_the_cause() {
    let fake = FakeHc::start();
    fake.set_terminals(vec![claude("t1", "blocked", "question")]);
    let hc = HcClient::new(HcSession::new(&fake.path), 10, 60);
    let pane = locate::pane(&hc.snapshot().unwrap(), "t1").unwrap().clone();

    let d = cards::answer_stop(&pane, &HerdrError::KeyGateStop);
    assert!(d.result == "changed" && d.message.as_deref().unwrap_or("").contains("another agent"), "another agent ({d:?})");

    let d = cards::answer_stop(&pane, &HerdrError::ScreenStop { keys_sent: false });
    let m = d.message.clone().unwrap_or_default();
    assert!(d.result == "screen_changed" && m.contains("Nothing was sent") && !m.contains("another agent"), "screen, no key ({d:?})");

    let d = cards::answer_stop(&pane, &HerdrError::ScreenStop { keys_sent: true });
    let m = d.message.clone().unwrap_or_default();
    assert!(
        d.result == "screen_changed" && m.contains("check the terminal") && !m.contains("Nothing was sent") && !m.contains("another agent"),
        "screen, keys in ({d:?})"
    );

    // hg-keygate-followups: another agent after some keys went in: not "nothing was sent".
    let d = cards::answer_stop(&pane, &HerdrError::KeyGateStopSent);
    let m = d.message.clone().unwrap_or_default();
    assert!(
        d.result == "changed" && m.contains("nother agent") && m.contains("Some keys may already be in: check the terminal") && !m.to_lowercase().contains("nothing was sent"),
        "another agent, keys in ({d:?})"
    );
}
