//! The shared Resources vectors (Tests/Fixtures/resources/vectors.json): the Mac app and the mod check the same
//! cases. The wants are the helper's: `FLOCKBOOK_WRITE_VECTORS=1 cargo test -p flockbook-modfeed --test
//! resources_vectors` writes them again after a rule change here.

use modfeedkit::{protocol, resources};
use serde_json::{json, Value};
use std::path::PathBuf;

const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../Tests/Fixtures/resources/vectors.json");

fn found(f: Vec<resources::Found>) -> Value {
    Value::Array(f.into_iter().map(|f| json!([f.start, f.end, f.kind, f.target])).collect())
}

/// A fresh real folder with `files` in it (the vector's `$ROOT`).
fn root_with(files: Option<&Value>) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    for (rel, body) in files.and_then(|f| f.as_object()).into_iter().flatten() {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body.as_str().unwrap()).unwrap();
    }
    (dir, root)
}

fn collected(v: &Value) -> Value {
    let (_dir, root) = root_with(v.get("files"));
    let r = root.to_string_lossy().to_string();
    let put = |s: &str| s.replace("$ROOT", &r);
    let back = |s: &str| s.replace(&r, "$ROOT");
    let log = root.join("session.jsonl");
    let lines: Vec<String> = v["lines"].as_array().unwrap().iter().map(|l| put(l.as_str().unwrap())).collect();
    std::fs::write(&log, lines.join("\n") + "\n").unwrap();
    let home = v.get("home").and_then(|h| h.as_str()).map(|h| PathBuf::from(put(h)));
    let agent = v.get("agent").and_then(|h| h.as_str()).map(|h| PathBuf::from(put(h)));
    let items = resources::answer_in(&log, None, home.as_deref(), agent.as_deref(), None, true).unwrap().0.items;
    Value::Array(
        items
            .iter()
            .map(|i| {
                let mut o = json!({"kind": i.kind, "target": back(&i.target), "title": back(&i.title), "detail": back(&i.detail), "row": i.row,
                    "first_ts": i.first_ts, "last_ts": i.last_ts, "mentions": i.mentions, "exists": i.exists});
                if !i.target.contains(&r) {
                    o["id"] = json!(i.id);
                }
                o
            })
            .collect(),
    )
}

fn computed(mut v: Value) -> Value {
    for c in v["detect"].as_array_mut().unwrap() {
        let t = c["text"].as_str().unwrap().to_string();
        c["feed"] = found(resources::detect(&t));
        c["collect"] = found(resources::detect_collect(&t));
    }
    for c in v["commands"].as_array_mut().unwrap() {
        let t = c["text"].as_str().unwrap().to_string();
        c["want"] = Value::Array(resources::commands(&t).into_iter().map(|c| json!([c.start, c.title, c.cmd])).collect());
    }
    for c in v["collect"].as_array_mut().unwrap() {
        c["want"] = collected(c);
    }
    v
}

#[test]
fn the_shared_vectors() {
    let have: Value = serde_json::from_str(&std::fs::read_to_string(PATH).unwrap()).unwrap();
    let got = computed(have.clone());
    if std::env::var_os("FLOCKBOOK_WRITE_VECTORS").is_some() {
        std::fs::write(PATH, serde_json::to_string_pretty(&got).unwrap() + "\n").unwrap();
        return;
    }
    for part in ["detect", "commands", "collect"] {
        for (h, g) in have[part].as_array().unwrap().iter().zip(got[part].as_array().unwrap()) {
            assert_eq!(h, g, "{part}: the helper gives another answer than vectors.json");
        }
    }
}

#[test]
fn ids_are_the_hash_of_the_key() {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(PATH).unwrap()).unwrap();
    let c = &v["collect"][1]["want"];
    let cmd = c.as_array().unwrap().iter().find(|i| i["target"] == "npm test").unwrap();
    assert_eq!(cmd["id"], protocol::fnv1a("cmd:npm test"));
}
