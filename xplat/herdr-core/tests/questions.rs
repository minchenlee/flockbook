//! Ports of ChecksQuestions.swift: AskUserQuestion, Codex questions, screen picker, answer keys.
mod common;
use common::*;
use herdr_core::conversation::ConversationParser;
use herdr_core::question::{self as qa, Step};
use herdr_core::{Delivery, ItemKind, QOption as Opt, Question, QuestionAnswer, QuestionRequest, QuestionState};

fn q_at(items: &[herdr_core::ConversationItem], i: usize) -> Option<&QuestionRequest> {
    match &items[i].kind {
        ItemKind::Question(r) => Some(r),
        _ => None,
    }
}
fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[test]
fn ask_user_question() {
    let items = parse(&fixture_lines("question.jsonl"));
    assert_eq!(describe(&items), ["asst:One question first.", "q:Branch", "q:Cleanup|Report", "q:Go", "asst:Last one.", "q:Ship"], "question: items");
    let (q1, q2) = (q_at(&items, 1).unwrap(), q_at(&items, 2).unwrap());
    assert!(
        q1.questions[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["integration", "main"]
            && q1.questions[0].options.iter().map(|o| o.recommended).collect::<Vec<_>>() == [true, false],
        "question: (Recommended) suffix removed and flagged"
    );
    assert!(q1.questions[0].options[0].detail == "Has the latest merged work." && !q1.questions[0].multi_select, "question: detail, single select");
    assert_eq!(q1.state, QuestionState::Answered(Some(s(&["integration (Recommended)"]))), "question: answers from toolUseResult");
    assert!(q2.questions[0].multi_select && q2.questions[0].options[1].label == "Build output" && q2.questions[0].options[1].recommended, "question: multiSelect, （建議） suffix");
    assert_eq!(q2.state, QuestionState::Answered(Some(s(&["Old caches, Build output（建議）", "Yes"]))), "question: answers parsed from result text, quotes escaped");
    assert_eq!(q_at(&items, 3).unwrap().state, QuestionState::Declined, "question: error result is .declined");
    assert!(q_at(&items, 5).unwrap().state == QuestionState::Pending && q_at(&items, 5).unwrap().tool_use_id == "toolu_q4", "question: no result yet is .pending");
    assert!(!items.iter().any(|i| matches!(i.kind, ItemKind::PermissionRequest(_))), "question: never a permissionRequest");
    let again: Vec<_> = parse(&fixture_lines("question.jsonl")).iter().map(|i| i.id.clone()).collect();
    assert!(items[5].id == "q-toolu_q4" && again == items.iter().map(|i| i.id.clone()).collect::<Vec<_>>(), "question: stable ids");
}

#[test]
fn codex_questions() {
    let cl = fixture_lines("question-codex.jsonl");
    let items = parse(&cl);
    assert_eq!(describe(&items), ["user:Ask me things.", "q:Color", "q:Size|Pet", "q:Go", "q:", "asst:Waiting for your pick.", "user:Safe", "q:Ship"], "codex question: items");
    let q = |i| q_at(&items, i).unwrap();
    assert!(
        q(1).agent == "codex" && q(1).delivery == Delivery::Picker && q(1).questions[0].id == "color"
            && q(1).questions[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["Red", "Blue"]
            && q(1).questions[0].options[0].recommended,
        "codex question: picker, id, (Recommended) flagged"
    );
    assert_eq!(q(1).state, QuestionState::Answered(Some(s(&["Blue"]))), "codex question: answer by id");
    assert_eq!(q(2).state, QuestionState::Answered(Some(s(&["Large", "一隻魚 fish"]))), "codex question: notes replace None of the above");
    assert_eq!(q(3).state, QuestionState::Declined, "codex question: unavailable output is .declined");
    assert!(
        q(4).delivery == Delivery::Message && q(4).questions[0].text == "Which plan?"
            && q(4).questions[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["Fast", "Safe"],
        "codex question: async title and string options"
    );
    assert_eq!(q(4).state, QuestionState::Answered(None), "codex question: async answered by the next user message");
    assert_eq!(q(7).state, QuestionState::Pending, "codex question: no output yet is .pending");
    assert!(!items.iter().any(|i| matches!(i.kind, ItemKind::PermissionRequest(_) | ItemKind::ToolChips(_))), "codex question: no permission, no tool chip");
    let mut p = ConversationParser::new();
    for l in &cl[..10] {
        p.feed(l);
    }
    assert!(
        p.items().iter().any(|i| matches!(&i.kind, ItemKind::Question(r) if r.delivery == Delivery::Message && r.state == QuestionState::Pending)),
        "codex question: async stays pending after {{accepted:true}}"
    );
}

#[test]
fn screen_question() {
    let single = "❯ Ask me.\n────────────────────\n ☐ Color\nPick a color?\n❯ 1. Red (Recommended)\n     warm\n  2. Blue\n     cool\n  3. Type something.\n────────────────────\n  4. Chat about this\nEnter to select · ↑/↓ to navigate · Esc to cancel";
    let (r_headers, r) = qa::claude_screen(single).expect("single picker");
    assert!(
        r_headers == ["Color"] && r.text == "Pick a color?"
            && r.options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["Red", "Blue"]
            && r.options.iter().map(|o| o.recommended).collect::<Vec<_>>() == [true, false]
            && r.options.iter().map(|o| o.detail.as_str()).collect::<Vec<_>>() == ["warm", "cool"]
            && !r.multi_select,
        "screen question: single"
    );
    let multi = "←  ☐ Size  ☒ Tops  ☐ Pay  ✔ Submit  →\nWhich toppings would you like\non top?\n❯ 1. [✔] Cheese\n     Melted\n  2. [ ] Olive\n  3. [ ] Type something\n     Submit\n────────────────────\n  4. Chat about this";
    let (m_headers, m) = qa::claude_screen(multi).expect("multi picker");
    assert!(
        m_headers == ["Size", "Tops", "Pay"] && m.text == "Which toppings would you like on top?" && m.multi_select
            && m.options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["Cheese", "Olive"],
        "screen question: multiSelect, wrapped text, tab bar"
    );
    assert!(
        qa::claude_screen("⏺ User answered Claude's questions:\n  ⎿  · Pick a color? → Blue").is_none()
            && qa::claude_screen("Do you want to proceed?\n❯ 1. Yes\n  2. No").is_none(),
        "screen question: none without a picker"
    );
}

#[test]
fn question_answer_keys() {
    let opts2 = || vec![Opt::new("A"), Opt::new("B")];
    let single = QuestionRequest::new("t", vec![Question::new("H", "Pick one?", opts2(), false)]);
    let multi = QuestionRequest::new("t", vec![Question::new("H", "Pick some?", opts2(), true)]);
    let three = QuestionRequest::new(
        "t",
        vec![Question::new("1", "First?", opts2(), false), Question::new("2", "Second?", opts2(), true), Question::new("3", "Third?", opts2(), false)],
    );
    let a = |picks: &[usize]| QuestionAnswer { picks: picks.to_vec(), other: None };
    let o = |t: &str| QuestionAnswer { picks: vec![], other: Some(t.into()) };
    let t = |x: &str| Step::Text(x.into());
    let k = |x: &[&str]| Step::Keys(s(x));
    let w = |x: &str| Step::WaitFor(x.into());

    assert_eq!(qa::steps(&single, &[a(&[1])]).unwrap(), [t("2")], "keys: claude single = digit");
    assert_eq!(qa::steps(&single, &[o("a fish")]).unwrap(), [t("3"), t("a fish"), k(&["Enter"])], "keys: claude single Other");
    let both = QuestionAnswer { picks: vec![1, 0], other: Some("tart".into()) };
    assert_eq!(
        qa::steps(&multi, &[both]).unwrap(),
        [t("1"), t("2"), t("3"), k(&["Down", "Down"]), t("tart"), k(&["Tab"]), k(&["Enter"]), w("Review your answers"), t("1")],
        "keys: claude multiSelect + Other"
    );
    assert_eq!(
        qa::steps(&three, &[a(&[0]), a(&[1]), o("x")]).unwrap(),
        [t("1"), w("Second?"), t("2"), k(&["Tab"]), w("Third?"), t("3"), t("x"), k(&["Enter"]), w("Review your answers"), t("1")],
        "keys: claude three questions"
    );
    let mut cx = three.clone();
    cx.agent = "codex".into();
    cx.questions[1].multi_select = false;
    assert_eq!(
        qa::steps(&cx, &[a(&[0]), a(&[1]), o("一隻魚")]).unwrap(),
        [t("1"), w("Second?"), t("2"), w("Third?"), k(&["Down", "Down"]), k(&["Tab"]), t("一隻魚"), k(&["Enter"])],
        "keys: codex digits, notes on None of the above"
    );
    let mut cxm = multi.clone();
    cxm.agent = "codex".into();
    let fails = |q: &QuestionRequest, ans: &[QuestionAnswer]| qa::steps(q, ans).is_err();
    assert!(
        fails(&cxm, &[a(&[0])]) && fails(&single, &[a(&[0, 1])]) && fails(&single, &[a(&[])]) && fails(&single, &[a(&[5])])
            && fails(&single, &[]) && fails(&single, &[o("two\nlines")]),
        "keys: unsupported answers throw before sending"
    );
    let screen = "←  ☐ H  ✔ Submit  →\nPick one?\n❯ 1. A\n  2. B\n  3. Type something.\n  4. Chat about this\nEnter to select";
    let echoed = format!("❯ Ask: question 'Pick some?' and 'Pick one?'\n{screen}");
    assert!(
        !qa::screen_shows(&multi, &echoed) && !qa::picker_region("claude", &echoed).unwrap().contains("Pick some?"),
        "keys: only the picker region counts, not the echoed prompt"
    );
    let cx_screen = "› Ask 'Cat or dog?' then 'Which size?'\n  Question 2/2 (1 unanswered)\n  Which size?\n  › 1. S\n  tab to add notes | enter to submit all";
    assert!(
        !qa::picker_region("codex", cx_screen).unwrap().contains("Cat or dog?")
            && qa::picker_region("codex", cx_screen).unwrap().contains("Which size?")
            && qa::picker_region("codex", "• Questions 1/1 answered\n  • Cat or dog?").is_none(),
        "keys: codex picker region"
    );
    assert!(
        qa::screen_shows(&single, screen) && !qa::screen_shows(&multi, screen)
            && !qa::screen_shows(&single, "⏺ User answered Claude's questions:\n  ⎿  · Pick one? → B"),
        "keys: screen check needs picker and question"
    );
    let mut asy = single.clone();
    asy.agent = "codex".into();
    asy.delivery = Delivery::Message;
    assert!(
        qa::message_text(&asy, &[a(&[1])]) == "B" && qa::message_text(&three, &[a(&[0]), a(&[0, 1]), o("x")]) == "First? A\nSecond? A, B\nThird? x",
        "keys: async message text"
    );
}
