//! The "Since you left" card and Summaries: the cases every side shares (Tests/HerdrKitTests/Fixtures/since-vectors.json).
//! The Mac app (Sources/HerdrKitChecks/ChecksSince.swift) and the mod (herdr-app/test/since-vectors.test.mjs) read the
//! same file: a rule changed on one side only fails here or there.

use modfeedkit::{glance, summary, tail};
use serde_json::Value;

fn vectors() -> Value {
    serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Tests/HerdrKitTests/Fixtures/since-vectors.json"))).unwrap()
}

fn lines(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|l| l.as_str().unwrap().to_string()).collect()
}

#[test]
fn test_names_and_first_sentences() {
    let v = vectors();
    for c in v["test_names"].as_array().unwrap() {
        assert_eq!(glance::test_name(c[0].as_str().unwrap()).as_deref(), c[1].as_str(), "{}", c[0]);
    }
    for c in v["first_sentence"].as_array().unwrap() {
        assert_eq!(glance::first_sentence(c[0].as_str().unwrap()).as_deref(), c[1].as_str(), "{}", c[0]);
    }
}

#[test]
fn the_facts_of_each_log() {
    for l in vectors()["logs"].as_array().unwrap() {
        let g = glance::facts(&tail::items(&lines(&l["lines"])), l["agent"].as_str().unwrap());
        let w = &l["want"];
        let name = l["name"].as_str().unwrap();
        assert_eq!((g.edited as u64, g.commands as u64), (w["edited"].as_u64().unwrap(), w["commands"].as_u64().unwrap()), "{name}");
        assert_eq!(g.failed.as_deref(), w["failed"].as_str(), "{name}");
        assert_eq!(g.tests.as_deref(), w["tests"].as_str(), "{name}");
        assert_eq!(g.test.as_deref(), w["test"].as_str(), "{name}");
        assert_eq!(g.said.as_deref(), w["said"].as_str(), "{name}");
    }
}

#[test]
fn the_summary_question_and_answer() {
    let v = vectors();
    for c in v["summary_parse"].as_array().unwrap() {
        let want = c[1].as_array().map(|a| (a[0].as_str().unwrap().to_string(), a[1].as_str().map(String::from)));
        assert_eq!(summary::parse(c[0].as_str().unwrap()), want, "{}", c[0]);
    }
    for c in v["model_name"].as_array().unwrap() {
        assert_eq!(summary::model_name(c[0].as_str().unwrap()).as_deref(), c[1].as_str(), "{}", c[0]);
    }
    let s = &v["summary_input"];
    let (_, text) = summary::input(&tail::items(&lines(&s["lines"])), s["agent"].as_str().unwrap()).unwrap();
    assert_eq!(text, s["text"].as_str().unwrap());
    assert_eq!(v["texts"]["model_empty"]["claude"].as_str(), Some(summary::CLAUDE_MODEL));
}
