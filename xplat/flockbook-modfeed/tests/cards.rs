//! The sidecar's cards on a fake herdr-core: read the card, then keys only for the
//! same request; every key gated.
#![cfg(unix)]

mod common;
use common::{claude, terminal, FakeHc};
use herdr_core::hc::{HcClient, HcSession};
use herdr_core::keygate::KeyGate;
use herdr_core::*;
use herdr_core::question;
use modfeedkit::{cards, locate};
use std::sync::{Arc, Mutex};

fn pane_of(hc: &HcClient, t: &str) -> Pane {
    locate::pane(&hc.snapshot().unwrap(), t).unwrap().clone()
}

const QUESTION: &str = "❯ Ask me.\n────────────────────\n ☐ Color\nPick a color?\n❯ 1. Red (Recommended)\n     warm\n  2. Blue\n     cool\n  3. Type something.\n────────────────────\n  4. Chat about this\nEnter to select · ↑/↓ to navigate · Esc to cancel";

fn picker(i: usize, other: bool) -> String {
    let tabs = if other { "←  ☐ Lunch  ☐ Drink  ✔ Submit  →" } else { "←  ☐ Color  ☐ Size  ✔ Submit  →" };
    let q = match (other, i) {
        (true, 0) => "Pick a lunch?\n❯ 1. Soup\n  2. Rice",
        (true, _) => "Pick a drink?\n❯ 1. Tea\n  2. Milk",
        (false, 0) => "Pick a color?\n❯ 1. Red\n  2. Blue",
        (false, _) => "Which size?\n❯ 1. S\n  2. M",
    };
    format!("{tabs}\n{q}\n  3. Type something.\n────────────────────\n  4. Chat about this\nEnter to select · Tab/Arrow keys to navigate · Esc to cancel")
}

#[test]
fn cards_on_a_fake_core() {
    let fake = FakeHc::start();
    fake.set_terminals(vec![claude("t1", "blocked", "permission")]);
    let hc = HcClient::new(HcSession::new(&fake.path), 10, 60);
    let pane = pane_of(&hc, "t1");

    // Approval with no log: the id is the prompt box's hash; a spinner above the box does not change it.
    fake.set_screen("t1", "✻ Working… 3s\n────────────────────\nBash command\n  rm -rf build\nDo you want to proceed?\n❯ 1. Yes\n  2. No");
    let c1 = cards::card(&pane, &hc, None, false, None);
    assert!(c1.kind.as_deref() == Some("approval") && c1.request.as_deref().unwrap().starts_with("screen-"), "approval card from the screen ({c1:?})");
    assert!(c1.command.as_deref().unwrap().contains("rm -rf build"), "the command from above the prompt");
    fake.set_screen("t1", "✻ Working… 9s\n────────────────────\nBash command\n  rm -rf build\nDo you want to proceed?\n❯ 1. Yes\n  2. No");
    let c2 = cards::card(&pane, &hc, None, false, None);
    assert_eq!(c2.request, c1.request, "same prompt, same id while the spinner moves");
    assert!(fake.keys().is_empty(), "reading sends no keys");

    let wrong = cards::approve(&pane, &hc, None, "screen-other", true, None);
    assert!(wrong.result == "changed" && fake.keys().is_empty(), "another request, nothing sent");
    let ok = cards::approve(&pane, &hc, None, c1.request.as_deref().unwrap(), true, None);
    assert!(ok.result == "sent" && fake.keys().last() == Some(&vec!["1".to_string()]), "allow sends '1' ({ok:?})");
    fake.set_screen("t1", "> done");
    let gone = cards::approve(&pane, &hc, None, c1.request.as_deref().unwrap(), false, None);
    assert!(gone.result == "changed" && fake.keys().len() == 1, "prompt gone, nothing sent");
    assert!(cards::card(&pane, &hc, None, false, None).kind.is_none(), "no prompt, no card");

    // Question: Claude's picker read from the screen.
    fake.set_terminals(vec![claude("t1", "blocked", "question")]);
    let qpane = pane_of(&hc, "t1");
    fake.set_screen("t1", QUESTION);
    let qc = cards::card(&qpane, &hc, None, false, None);
    let q0 = &qc.question.as_ref().unwrap().questions[0];
    assert!(
        qc.kind.as_deref() == Some("question") && q0.options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["Red", "Blue"] && q0.options[0].recommended,
        "question card ({qc:?})"
    );
    let k0 = fake.keys().len();
    let qa = cards::card(&qpane, &hc, None, true, None);
    assert!(qa == qc && fake.keys().len() == k0, "card --all: one question, same card, no keys");

    // card --all walks the picker with a gate: another program took the pane after the read, not one key goes out.
    fake.set_terminals(vec![claude("t1", "blocked", "question")]);
    let mpane = pane_of(&hc, "t1");
    fake.set_screen("t1", "←  ☐ Color  ☐ Size  ✔ Submit  →\nPick a color?\n❯ 1. Red\n  2. Blue\n  3. Type something.\n────────────────────\n  4. Chat about this\nEnter to select · Tab/Arrow keys to navigate · Esc to cancel");
    fake.set_terminals(vec![terminal("t1", &["codex"], Some(5151), Some("blocked"), Some("question"))]);
    let k1 = fake.keys().len();
    let _ = cards::card(&mpane, &hc, None, true, None);
    assert_eq!(fake.keys().len(), k1, "card --all: gated, another agent in the pane, no keys");
    // The Mac's rule goes through the same gate.
    let r = question::load_all_questions(&hc, &mpane.id, Some(&KeyGate::on_mac(mpane.agent.clone(), Some(Arc::new(|| true)))), None);
    assert!(matches!(r, Err(HerdrError::KeyGateStop)) && fake.keys().len() == k1, "mac load all: gated, another agent, stopped ({r:?})");

    // The Mac: an agent with no identity is not refused; each key checks the screen instead.
    let sim = Arc::new(Mutex::new((0usize, false)));
    let mut bare = mpane.clone();
    bare.agent.as_mut().unwrap().process = None;
    bare.agent.as_mut().unwrap().session_id = None;
    fake.set_terminals(vec![claude("t1", "blocked", "question")]);
    fake.set_screen("t1", &picker(0, false));
    let s2 = sim.clone();
    fake.on_keys(Some(Arc::new(move |a, k, st| {
        if a != "t1" {
            return;
        }
        let mut s = s2.lock().unwrap();
        if k == ["left"] {
            s.0 = s.0.saturating_sub(1);
        } else if k == ["right"] {
            s.0 = (s.0 + 1).min(1);
        }
        st.screens.insert("t1".into(), picker(s.0, s.1));
    })));
    let one = question::pending_question(&hc, &mpane.id).ok().flatten();
    let mac_all = question::load_all_questions(&hc, &mpane.id, Some(&KeyGate::on_mac(bare.agent.clone(), Some(Arc::new(|| true)))), one.as_ref());
    let mac_all = mac_all.ok().flatten();
    assert_eq!(mac_all.as_ref().map(|q| q.questions.len()), Some(2), "mac load all: no agent identity, still reads every question");
    // Another request comes up after the first key: the next key's screen check stops the walk.
    *sim.lock().unwrap() = (0, false);
    fake.set_screen("t1", &picker(0, false));
    fake.on_keys(Some(Arc::new(|a, _k, st| {
        if a == "t1" {
            st.screens.insert("t1".into(), picker(0, true));
        }
    })));
    let ka = fake.keys().len();
    let switched = question::load_all_questions(&hc, &mpane.id, Some(&KeyGate::on_mac(bare.agent.clone(), Some(Arc::new(|| true)))), one.as_ref());
    assert!(switched.is_err() && fake.keys().len() - ka <= 1, "mac load all: another question on screen, stopped ({switched:?})");
    // The Mac's answer keys: another agent (with an identity) in the pane, not one key.
    fake.on_keys(None);
    fake.set_screen("t1", &picker(0, false));
    fake.set_terminals(vec![terminal("t1", &["codex"], Some(5151), Some("blocked"), Some("question"))]);
    let kb = fake.keys().len();
    let full = mac_all.unwrap();
    let ans = [QuestionAnswer { picks: vec![0], other: None }, QuestionAnswer { picks: vec![0], other: None }];
    let r = question::answer_question(&hc, &mpane.id, &full, &ans, Some(&KeyGate::on_mac(mpane.agent.clone(), Some(Arc::new(|| true)))));
    assert!(matches!(r, Err(HerdrError::KeyGateStop)) && fake.keys().len() == kb, "mac answer: gated, another agent, no keys");
    // The same agent: the gate lets the walk start.
    fake.set_terminals(vec![claude("t1", "blocked", "question")]);
    fake.set_screen("t1", &picker(0, false));
    let _ = cards::card(&mpane, &hc, None, true, None);
    assert_eq!(fake.keys().get(k1), Some(&vec!["left".to_string()]), "card --all: gated, same agent, the walk starts ({:?})", fake.keys());

    fake.set_screen("t1", QUESTION);
    let bad = cards::answer(&qpane, &hc, None, "screen-nope", &[QuestionAnswer { picks: vec![1], other: None }]);
    assert_eq!(bad.result, "changed", "answer: another request, nothing sent");

    // Every key is gated: another program in the pane after the read, nothing goes out, and the reply says why.
    let sends = || fake.calls("terminal.send").len();
    let (g0, s0) = (fake.keys().len(), sends());
    fake.set_terminals(vec![terminal("t1", &["codex"], Some(5151), Some("blocked"), Some("question"))]);
    let ga = cards::answer(&qpane, &hc, None, qc.request.as_deref().unwrap(), &[QuestionAnswer { picks: vec![1], other: None }]);
    assert!(ga.result == "changed" && ga.message.as_deref().unwrap().contains("agent") && fake.keys().len() == g0, "answer: gated ({ga:?})");
    let gs = cards::send(&qpane, &hc, "hello", &[], None, None, None, None);
    assert!(gs.result == "changed" && gs.message.as_deref().unwrap().contains("agent") && sends() == s0, "send: gated ({gs:?})");
    fake.set_terminals(vec![claude("t1", "blocked", "permission")]);
    let apane = pane_of(&hc, "t1");
    fake.set_screen("t1", "Bash command\n  rm -rf build\nDo you want to proceed?\n❯ 1. Yes\n  2. No");
    let ac = cards::card(&apane, &hc, None, false, None);
    fake.set_terminals(vec![terminal("t1", &["codex"], Some(5151), Some("blocked"), Some("permission"))]);
    let gp = cards::approve(&apane, &hc, None, ac.request.as_deref().unwrap_or(""), true, None);
    assert!(gp.result == "changed" && gp.message.as_deref().unwrap().contains("agent") && fake.keys().len() == g0, "approve: gated ({gp:?})");

    fake.set_terminals(vec![claude("t1", "blocked", "question")]);
    fake.set_screen("t1", QUESTION);
    let before = fake.keys().len();
    fake.on_keys(Some(Arc::new(|a, k, st| {
        if a == "t1" && k == ["2"] {
            st.screens.insert("t1".into(), "⏺ User answered: Blue\n> ".into());
        }
    })));
    let answered = cards::answer(&qpane, &hc, None, qc.request.as_deref().unwrap(), &cards::parse_answers(r#"[{"picks":[1]}]"#).unwrap());
    fake.on_keys(None);
    assert!(answered.result == "sent" && fake.keys()[before..] == [vec!["2".to_string()]], "answer: the picker keys ({answered:?})");
    assert_eq!(cards::parse_answers(r#"[{"picks":[],"other":"teal"}]"#), Some(vec![QuestionAnswer { picks: vec![], other: Some("teal".into()) }]));
    assert!(cards::parse_answers("{}").is_none() && cards::parse_answers(r#"[{"picks":"x"}]"#).is_none(), "bad JSON refused");

    let sent = cards::send(&qpane, &hc, "hello", &[], None, None, None, None);
    let call = fake.calls("terminal.send").last().cloned().unwrap();
    assert!(sent.result == "sent" && call["text"] == "hello" && call["terminal"] == "t1", "send: terminal.send to the terminal ({sent:?})");
}
