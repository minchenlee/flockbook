//! Links in the chat: the cases the helper and the mod share (Tests/Fixtures/resources/md-links.json). The helper's
//! block split must match the mod's `splitBlocks` (`blocks`), and a link gets `block` (the mod writes it as a
//! Markdown link) exactly for `wrapped`. `links` is what the helper sends; herdr-app/test/links.test.mjs draws the
//! same file. MD_LINKS_UPDATE=1 writes `links` from this helper (look at the diff before you keep it).

use modfeedkit::mdsplit::{judge, md_blocks};
use modfeedkit::resources::links;
use serde_json::{json, Value};

const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../Tests/Fixtures/resources/md-links.json");

#[test]
fn the_mod_and_the_helper_agree() {
    let mut cases: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(PATH).unwrap()).unwrap();
    let update = std::env::var_os("MD_LINKS_UPDATE").is_some();
    let mut bad = vec![];
    for c in &mut cases {
        let name = c["name"].as_str().unwrap().to_string();
        let text = c["text"].as_str().unwrap().to_string();
        let starts: Vec<usize> = md_blocks(&text).iter().map(|r| text[..r.start].encode_utf16().count()).collect();
        let want: Vec<usize> = c["blocks"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as usize).collect();
        if starts != want {
            bad.push(format!("{name}: blocks {starts:?}, the mod's {want:?}"));
        }
        let got = judge(&text, links(&text));
        let u16: Vec<u16> = text.encode_utf16().collect();
        let wrapped: Vec<String> = got.iter().filter(|l| l.block.is_some()).map(|l| String::from_utf16(&u16[l.start..l.end]).unwrap()).collect();
        let want_w: Vec<String> = c["wrapped"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
        if wrapped != want_w {
            bad.push(format!("{name}: wrapped {wrapped:?}, want {want_w:?}"));
        }
        for l in &got {
            if let Some(b) = l.block {
                if !want.contains(&b) {
                    bad.push(format!("{name}: block {b} is not one of the mod's"));
                }
            }
        }
        let sent = serde_json::to_value(&got).unwrap();
        if update {
            c["links"] = sent;
        } else if c.get("links") != Some(&sent) {
            bad.push(format!("{name}: links {sent}, the file has {}", c.get("links").unwrap_or(&json!(null))));
        }
    }
    if update {
        std::fs::write(PATH, serde_json::to_string_pretty(&cases).unwrap() + "\n").unwrap();
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
