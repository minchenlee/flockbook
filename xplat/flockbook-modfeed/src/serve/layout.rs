//! Spaces and tabs as the Mac app shows them (`HCMapper.workspaces`, `HCAppDoc`, `AppDocNames`): each machine's
//! `herdr-app` document (`doc.get`, `doc.get {on}`), else the terminals' `herdr.space` / `herdr.tab` labels, else
//! "Other terminals". Terminals on other machines (`box/t3`) are grouped by their own machine's document; its ids are
//! shown qualified (`box/sp1`). Read only: this never writes a document.

use herdr_core::hc::mapper;
use herdr_core::hc::HcClient;
use herdr_core::{AgentStatus, Pane, Tab, Workspace};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const APP_DOC: &str = "herdr-app";
pub const OTHER_ID: &str = "hc:other";
pub const OTHER_LABEL: &str = "Other terminals";
/// Seconds one machine's `doc.get` may take; how old the other machines' documents may get; how long a snapshot
/// waits for a running read of them (the Mac app's values).
const MACHINE_DOC_TIMEOUT: Duration = Duration::from_secs(3);
const MACHINE_DOCS_TTL: Duration = Duration::from_secs(2);
const MACHINE_DOCS_WAIT: Duration = Duration::from_millis(250);

/// A `herdr-app` document's spaces, when its shape is the one the official app writes (else none: labels apply).
pub struct Doc {
    spaces: Vec<Value>,
}

const NODE_TYPES: [&str; 4] = ["block", "row", "column", "stack"];

fn string_or_absent(o: &Value, k: &str) -> bool {
    o.get(k).is_none_or(Value::is_string)
}

fn node_ok(n: &Value, depth: usize) -> bool {
    if depth >= 64 || !n.is_object() || !string_or_absent(n, "id") {
        return false;
    }
    let Some(ty) = n["type"].as_str().filter(|t| NODE_TYPES.contains(t)) else { return false };
    let shape = if ty == "block" {
        n["content"].is_object() && n["content"].get("terminal").is_none_or(Value::is_string)
    } else {
        n["children"].as_array().is_some_and(|c| c.iter().all(|c| node_ok(c, depth + 1)))
    };
    let size = n.get("size").is_none_or(|s| s["px"].is_number() || s["weight"].is_number());
    shape && size
}

fn space_ok(s: &Value) -> bool {
    if !s.is_object() || !["id", "name", "folder"].iter().all(|k| string_or_absent(s, k)) {
        return false;
    }
    match s.get("tabs") {
        None => true,
        Some(Value::Array(a)) => {
            a.iter().all(|t| t.is_object() && string_or_absent(t, "id") && string_or_absent(t, "name") && t.get("root").is_none_or(|r| node_ok(r, 0)))
        }
        Some(_) => false,
    }
}

impl Doc {
    /// `value` as `doc.get` gave it. Null: no document yet (no spaces). None: a shape this does not know.
    pub fn parse(value: &Value) -> Option<Doc> {
        match value {
            Value::Null => Some(Doc { spaces: vec![] }),
            Value::Object(o) => match o.get("spaces") {
                None | Some(Value::Null) => Some(Doc { spaces: vec![] }),
                Some(Value::Array(a)) if a.iter().all(space_ok) => Some(Doc { spaces: a.clone() }),
                _ => None,
            },
            _ => None,
        }
    }

    fn space_ids(&self) -> Vec<String> {
        self.spaces.iter().filter_map(|s| s["id"].as_str().map(String::from)).collect()
    }

    fn tabs(&self, si: usize) -> &[Value] {
        self.spaces[si]["tabs"].as_array().map_or(&[], |a| a.as_slice())
    }

    fn tab_ids(&self, space: &str) -> Vec<String> {
        let Some(si) = self.spaces.iter().position(|s| s["id"] == space) else { return vec![] };
        self.tabs(si).iter().filter_map(|t| t["id"].as_str().map(String::from)).collect()
    }

    fn tab_at(&self, id: &str) -> Option<(usize, usize)> {
        (0..self.spaces.len()).find_map(|si| self.tabs(si).iter().position(|t| t["id"] == id).map(|ti| (si, ti)))
    }

    fn tab_position(&self, id: &str) -> Option<usize> {
        self.tab_at(id).map(|(_, ti)| ti)
    }

    /// Pane blocks (terminal or mod) in tree order.
    fn blocks(n: &Value, out: &mut Vec<Value>) {
        let c = &n["content"];
        if n["type"] == "block" && (c["terminal"].is_string() || c["mod"].is_string()) {
            out.push(n.clone());
        }
        for ch in n["children"].as_array().map_or(&[][..], |a| a.as_slice()) {
            Self::blocks(ch, out);
        }
    }

    fn tab_blocks(t: &Value) -> Vec<Value> {
        let mut out = vec![];
        if t["root"].is_object() {
            Self::blocks(&t["root"], &mut out);
        }
        out
    }

    /// Terminal addresses of a tab's panes, in tree order.
    fn terminals(&self, tab: &str) -> Vec<String> {
        let Some((si, ti)) = self.tab_at(tab) else { return vec![] };
        Self::tab_blocks(&self.tabs(si)[ti]).iter().filter_map(|b| b["content"]["terminal"].as_str().map(String::from)).collect()
    }

    /// The block showing `terminal`, and its (space, tab).
    fn block(&self, terminal: &str) -> Option<(Value, String, String)> {
        for (si, s) in self.spaces.iter().enumerate() {
            for t in self.tabs(si) {
                if let Some(b) = Self::tab_blocks(t).into_iter().find(|b| b["content"]["terminal"] == terminal) {
                    return Some((b, s["id"].as_str().unwrap_or("").into(), t["id"].as_str().unwrap_or("").into()));
                }
            }
        }
        None
    }

    fn place(&self, terminal: &str) -> Option<(String, String)> {
        self.block(terminal).map(|(_, s, t)| (s, t))
    }

    fn pane_title(&self, terminal: &str) -> Option<String> {
        self.block(terminal).and_then(|(b, _, _)| b["title"].as_str().filter(|t| !t.is_empty()).map(String::from))
    }
}

/// Space and tab names out of a document of any shape (`AppDocNames`): any object with a string id (or keyed by id)
/// and a string `name`, `label` or `title`. An ancestor key with "tab" files it under tabs, "space" under spaces.
#[derive(Default)]
pub struct Names {
    spaces: HashMap<String, String>,
    tabs: HashMap<String, String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    None,
    Space,
    Tab,
}

fn display_name(o: &serde_json::Map<String, Value>) -> Option<String> {
    ["name", "label", "title"].iter().find_map(|k| o.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from))
}

impl Names {
    pub fn of(doc: &Value) -> Names {
        let mut n = Names::default();
        n.walk(doc, Ctx::None, None, 0);
        n
    }

    fn walk(&mut self, v: &Value, ctx: Ctx, key_id: Option<&str>, depth: usize) {
        if depth >= 12 {
            return;
        }
        match v {
            Value::Object(o) => {
                if let (Some(id), Some(name)) = (o.get("id").and_then(Value::as_str).or(key_id), display_name(o)) {
                    if ctx != Ctx::Tab {
                        self.spaces.entry(id.into()).or_insert_with(|| name.clone());
                    }
                    if ctx != Ctx::Space {
                        self.tabs.entry(id.into()).or_insert(name);
                    }
                }
                for (k, child) in o {
                    let lk = k.to_lowercase();
                    let c = if lk.contains("tab") {
                        Ctx::Tab
                    } else if lk.contains("space") {
                        Ctx::Space
                    } else {
                        ctx
                    };
                    let container = lk.contains("tab") || lk.contains("space");
                    match child {
                        Value::Object(m) if container && m.get("id").is_none() && display_name(m).is_none() => {
                            for (id, item) in m {
                                self.walk(item, c, Some(id), depth + 2);
                            }
                        }
                        _ => self.walk(child, c, None, depth + 1),
                    }
                }
            }
            Value::Array(a) => {
                for item in a {
                    self.walk(item, ctx, None, depth + 1);
                }
            }
            _ => {}
        }
    }
}

/// `box/` for `box/t7`, else "".
fn prefix(address: &str) -> &str {
    address.rfind('/').map_or("", |i| &address[..=i])
}

/// (machine prefix, local id) of a qualified id.
fn local(id: &str) -> (&str, &str) {
    let m = prefix(id);
    (m, &id[m.len()..])
}

/// An id the official app made: a short prefix and 32 hex digits.
fn fresh_id(id: &str) -> bool {
    ["sp", "tb", "p", "s"].iter().any(|p| id.len() == p.len() + 32 && id.starts_with(p) && id[p.len()..].bytes().all(|b| b.is_ascii_hexdigit()))
}

fn folder_name(path: &str) -> Option<String> {
    let p = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    (!p.is_empty()).then(|| p.to_string())
}

/// `items` in their place in `order`; the rest keep their order, after them.
fn ordered<T>(items: Vec<T>, order: &[String], key: impl Fn(&T) -> String) -> Vec<T> {
    if order.is_empty() {
        return items;
    }
    let rank: HashMap<&str, usize> = order.iter().enumerate().rev().map(|(i, k)| (k.as_str(), i)).collect();
    let mut v: Vec<(usize, T)> = items.into_iter().map(|t| (rank.get(key(&t).as_str()).copied().unwrap_or(usize::MAX), t)).collect();
    v.sort_by_key(|(r, _)| *r); // stable
    v.into_iter().map(|(_, t)| t).collect()
}

/// `terms`: `terminal.list` records. `app_doc`: this machine's `herdr-app` value. `machine_docs`: each other
/// machine's, in the order to show them.
pub fn workspaces(terms: &[Value], app_doc: &Value, machine_docs: &[(String, Value)]) -> Vec<Workspace> {
    let mut names: HashMap<String, Names> = HashMap::from([(String::new(), Names::of(app_doc))]);
    let mut docs: HashMap<String, Doc> = HashMap::new();
    if let Some(d) = Doc::parse(app_doc) {
        docs.insert(String::new(), d);
    }
    for (m, v) in machine_docs {
        names.insert(format!("{m}/"), Names::of(v));
        if let Some(d) = Doc::parse(v) {
            docs.insert(format!("{m}/"), d);
        }
    }
    // The panes as the sidecar maps them (live only), and each terminal's record for its labels and parent.
    let panes: HashMap<String, Pane> = mapper::workspaces(terms).into_iter().flat_map(|w| w.tabs).flat_map(|t| t.panes).map(|p| (p.terminal_id.clone(), p)).collect();
    let live: Vec<&Value> = terms.iter().filter(|t| t["address"].as_str().is_some_and(|a| panes.contains_key(a))).collect();
    let by_addr: HashMap<&str, &Value> = live.iter().rev().map(|t| (t["address"].as_str().unwrap_or(""), *t)).collect();
    let label = |t: &Value, k: &str| t["labels"][k].as_str().filter(|s| !s.is_empty()).map(String::from);

    // (space, tab): where its machine's document shows it, else its labels, else the nearest such ancestor's.
    let group = |t: &Value| -> Option<(String, String)> {
        let mut cur = Some(t);
        let mut seen = std::collections::HashSet::new();
        while let Some(c) = cur {
            let addr = c["address"].as_str().unwrap_or("");
            if !seen.insert(addr.to_string()) {
                break;
            }
            let (m, own) = local(addr);
            if let Some((s, tb)) = docs.get(m).and_then(|d| d.place(own)) {
                return Some((format!("{m}{s}"), format!("{m}{tb}")));
            }
            if let Some(s) = label(c, "herdr.space") {
                return Some((format!("{m}{s}"), format!("{m}{}", label(c, "herdr.tab").unwrap_or_else(|| format!("@{own}")))));
            }
            // A parent is named on the child's own machine: `box/t3` before a local `t3`.
            cur = c["parent"].as_str().filter(|p| !p.is_empty()).and_then(|p| by_addr.get(format!("{m}{p}").as_str()).or_else(|| by_addr.get(p)).copied());
        }
        None
    };

    let mut space_order: Vec<String> = vec![];
    let mut tab_order: HashMap<String, Vec<String>> = HashMap::new();
    let mut grouped: HashMap<(String, String), Vec<&Value>> = HashMap::new();
    let mut others: Vec<&Value> = vec![];
    for t in &live {
        let Some((s, tb)) = group(t) else {
            others.push(t);
            continue;
        };
        let tabs = tab_order.entry(s.clone()).or_insert_with(|| {
            space_order.push(s.clone());
            vec![]
        });
        if !tabs.contains(&tb) {
            tabs.push(tb.clone());
        }
        grouped.entry((s, tb)).or_default().push(t);
    }
    if !docs.is_empty() {
        // This machine first, then the others in the order given.
        let prefixes: Vec<String> = std::iter::once(String::new()).chain(machine_docs.iter().map(|(m, _)| format!("{m}/"))).collect();
        let order: Vec<String> = prefixes.iter().flat_map(|m| docs.get(m).map(|d| d.space_ids().into_iter().map(|s| format!("{m}{s}")).collect::<Vec<_>>()).unwrap_or_default()).collect();
        space_order = ordered(space_order, &order, String::clone);
        for s in &space_order {
            let (m, own) = local(s);
            if let Some(d) = docs.get(m) {
                let order: Vec<String> = d.tab_ids(own).into_iter().map(|t| format!("{m}{t}")).collect();
                let tabs = tab_order.remove(s).unwrap_or_default();
                tab_order.insert(s.clone(), ordered(tabs, &order, String::clone));
            }
        }
        for ((_, tb), ts) in grouped.iter_mut() {
            let (m, own) = local(tb);
            if let Some(d) = docs.get(m) {
                let order: Vec<String> = d.terminals(own).into_iter().map(|t| format!("{m}{t}")).collect();
                *ts = ordered(std::mem::take(ts), &order, |t| t["address"].as_str().unwrap_or("").to_string());
            }
        }
    }

    let make_tab = |id: String, label: String, number: usize, ts: &[&Value]| -> Tab {
        let ps: Vec<Pane> = ts
            .iter()
            .filter_map(|t| {
                let addr = t["address"].as_str()?;
                let mut p = panes.get(addr)?.clone();
                let (m, own) = local(addr);
                if let Some(title) = docs.get(m).and_then(|d| d.pane_title(own)) {
                    p.label = Some(title);
                }
                Some(p)
            })
            .collect();
        let status = AgentStatus::rollup(ps.iter().filter_map(|p| p.agent.as_ref().map(|a| a.status)));
        Tab { id, label, number: number as i64, panes: ps, status }
    };

    let mut out: Vec<Workspace> = space_order
        .iter()
        .map(|s| {
            let tabs: Vec<Tab> = tab_order[s]
                .iter()
                .enumerate()
                .map(|(i, tb)| {
                    // An unnamed tab is called by its place in the document; one only labeled, by its shown number.
                    let (m, own) = local(tb);
                    let label = names.get(m).and_then(|n| n.tabs.get(own).cloned()).or_else(|| docs.get(m).and_then(|d| d.tab_position(own)).map(|p| (p + 1).to_string())).unwrap_or_else(|| {
                        if fresh_id(own) || own.starts_with('@') {
                            (i + 1).to_string()
                        } else {
                            own.to_string()
                        }
                    });
                    make_tab(format!("hc:tab:{s}/{own}"), label, i + 1, grouped.get(&(s.clone(), tb.clone())).map_or(&[][..], |v| v.as_slice()))
                })
                .collect();
            let (m, own) = local(s);
            let first = tab_order[s].first().and_then(|tb| grouped.get(&(s.clone(), tb.clone()))).and_then(|v| v.first()).and_then(|t| panes.get(t["address"].as_str().unwrap_or("")));
            let label = names.get(m).and_then(|n| n.spaces.get(own).cloned()).or_else(|| first.and_then(|p| folder_name(&p.cwd))).unwrap_or_else(|| s.clone());
            Workspace { id: format!("hc:space:{s}"), label, tabs }
        })
        .collect();
    if !others.is_empty() {
        let tabs = others
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let addr = t["address"].as_str().unwrap_or("");
                let title = t["title"].as_str().filter(|s| !s.is_empty());
                let cwd = panes.get(addr).map(|p| p.cwd.clone()).unwrap_or_default();
                let label = t["name"].as_str().filter(|s| !s.is_empty()).or(title).map(String::from).or_else(|| folder_name(&cwd)).unwrap_or_else(|| addr.to_string());
                make_tab(format!("hc:term:{addr}"), label, i + 1, &[*t])
            })
            .collect();
        out.push(Workspace { id: OTHER_ID.into(), label: OTHER_LABEL.into(), tabs });
    }
    out
}

/// The other machines' documents: the last read, when, and whether a read runs now.
struct MachineDocs {
    docs: Vec<(String, Value)>,
    at: Option<Instant>,
    running: bool,
}

static MACHINE_DOCS: Mutex<MachineDocs> = Mutex::new(MachineDocs { docs: vec![], at: None, running: false });

/// Each connected, kept machine's document (`machine.list` order). One that cannot be read keeps its last one;
/// a failed `machine.list` keeps them all.
fn read_machine_docs(client: &HcClient, previous: &[(String, Value)]) -> Vec<(String, Value)> {
    let Ok(list) = client.call("machine.list", json!({}), Some(MACHINE_DOC_TIMEOUT)) else { return previous.to_vec() };
    let names: Vec<String> = list["machines"]
        .as_array()
        .map_or(&[][..], |a| a.as_slice())
        .iter()
        .filter(|m| m["state"] == "connected" && m["temporary"] != true)
        .filter_map(|m| m["name"].as_str().filter(|n| !n.is_empty()).map(String::from))
        .collect();
    let got: Vec<Option<Value>> = std::thread::scope(|sc| {
        let hs: Vec<_> = names
            .iter()
            .map(|n| {
                let session = herdr_core::hc::HcSession::resolve_env();
                sc.spawn(move || HcClient::new(session, 8, 22).call("doc.get", json!({"name": APP_DOC, "on": n}), Some(MACHINE_DOC_TIMEOUT)).ok().map(|r| r["value"].clone()))
            })
            .collect();
        hs.into_iter().map(|h| h.join().ok().flatten()).collect()
    });
    names.into_iter().zip(got).filter_map(|(n, d)| d.or_else(|| previous.iter().find(|(m, _)| *m == n).map(|(_, v)| v.clone())).map(|d| (n, d))).collect()
}

/// The other machines' documents now: read in the background when older than the TTL (a slow machine never holds up a
/// snapshot); a quick read shows in this snapshot, a slow one in a later one.
fn machine_docs() -> Vec<(String, Value)> {
    machine_docs_with(|previous| {
        let client = HcClient::new(herdr_core::hc::HcSession::resolve_env(), 8, 22);
        read_machine_docs(&client, previous)
    })
}

/// Ends a background read: `running` goes back to false even when the read panics, and then the next snapshot
/// starts a new read at once (`at` cleared), so the layout never stops refreshing.
struct ReadDone {
    docs: Option<Vec<(String, Value)>>,
}

impl Drop for ReadDone {
    fn drop(&mut self) {
        let mut g = MACHINE_DOCS.lock().unwrap_or_else(|e| e.into_inner());
        match self.docs.take() {
            Some(d) => g.docs = d,
            None => g.at = None,
        }
        g.running = false;
    }
}

fn machine_docs_with(read: impl FnOnce(&[(String, Value)]) -> Vec<(String, Value)> + Send + 'static) -> Vec<(String, Value)> {
    let lock = || MACHINE_DOCS.lock().unwrap_or_else(|e| e.into_inner());
    let start = {
        let mut g = lock();
        let stale = g.at.is_none_or(|a| a.elapsed() > MACHINE_DOCS_TTL);
        if stale && !g.running {
            g.running = true;
            g.at = Some(Instant::now());
            Some(g.docs.clone())
        } else {
            None
        }
    };
    if let Some(previous) = start {
        std::thread::spawn(move || {
            let mut done = ReadDone { docs: None };
            done.docs = Some(read(&previous));
        });
        let until = Instant::now() + MACHINE_DOCS_WAIT;
        while Instant::now() < until && lock().running {
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    lock().docs.clone()
}

/// `terminal.list` and the documents, grouped. Err: `terminal.list` failed. `remote_never` (the person chose Never for
/// the helper on other machines): their documents are not read either, and their tabs group by labels.
pub fn snapshot(client: &HcClient, remote_never: bool) -> Result<Vec<Workspace>, herdr_core::HerdrError> {
    let list = client.call("terminal.list", json!({}), None)?;
    let terms = list["terminals"].as_array().cloned().unwrap_or_default();
    // A document that cannot be read: names fall back to labels and folders.
    let app = client.call("doc.get", json!({"name": APP_DOC}), None).map(|r| r["value"].clone()).unwrap_or(Value::Null);
    Ok(workspaces(&terms, &app, &if remote_never { vec![] } else { machine_docs() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(addr: &str, space: &str, tab: &str) -> Value {
        json!({"address": addr, "labels": {"herdr.space": space, "herdr.tab": tab}, "argv": ["zsh"], "cwd": "/w/proj", "state": "live", "pid": 11})
    }

    /// One space `sp1` with tab `tb1` holding the given terminals, in that order, with pane titles.
    fn one_space(name: &str, tab: &str, terms: &[(&str, &str)]) -> Value {
        let kids: Vec<Value> = terms.iter().enumerate().map(|(i, (t, title))| json!({"id": format!("p{i}"), "type": "block", "content": {"terminal": t}, "title": title})).collect();
        json!({"spaces": [{"id": "sp1", "name": name, "folder": "/w", "tabs": [{"id": "tb1", "name": tab, "root": {"id": "r", "type": "row", "children": kids}}]}]})
    }

    fn ids(ws: &[Workspace]) -> Vec<&str> {
        ws.iter().map(|w| w.id.as_str()).collect()
    }

    #[test]
    fn each_machine_by_its_own_document() {
        let terms = vec![term("t1", "sp1", "tb1"), term("box/t3", "sp1", "tb1"), term("box/t4", "sp1", "tb1")];
        let here = one_space("Here", "main", &[("t1", "")]);
        let there = one_space("Remote", "build", &[("t4", "first"), ("t3", "second")]);
        let ws = workspaces(&terms, &here, &[("box".into(), there)]);
        assert_eq!(ids(&ws), ["hc:space:sp1", "hc:space:box/sp1"], "one space per machine, qualified");
        assert_eq!(ws.iter().map(|w| w.label.as_str()).collect::<Vec<_>>(), ["Here", "Remote"]);
        let rt = &ws[1].tabs[0];
        assert_eq!((rt.id.as_str(), rt.label.as_str()), ("hc:tab:box/sp1/tb1", "build"));
        assert_eq!(rt.panes.iter().map(|p| p.terminal_id.as_str()).collect::<Vec<_>>(), ["box/t4", "box/t3"], "pane order from that machine's document");
        assert_eq!(rt.panes.iter().map(|p| p.label.clone().unwrap_or_default()).collect::<Vec<_>>(), ["first", "second"]);
        assert_eq!(rt.panes[0].id, "hc:box/t4", "pane ids as the sidecar's");
        assert_eq!(ws[0].tabs[0].panes.iter().map(|p| p.terminal_id.as_str()).collect::<Vec<_>>(), ["t1"]);
        // Without that document: its labels keep the machines apart.
        let bare = workspaces(&terms, &here, &[]);
        assert_eq!(ids(&bare), ["hc:space:sp1", "hc:space:box/sp1"]);
        assert_eq!(bare[1].label, "proj", "a space with no name: its first pane's folder");
    }

    #[test]
    fn labels_parents_and_others() {
        let mut child = json!({"address": "t2", "argv": ["claude"], "cwd": "/w", "state": "live", "parent": "t1", "pid": 7, "status": {"state": "blocked", "kind": "permission"}});
        child["labels"] = json!({});
        let lone = json!({"address": "t9", "argv": ["zsh"], "cwd": "/home/you/notes", "state": "live", "name": "scratch"});
        let gone = json!({"address": "t5", "argv": ["zsh"], "cwd": "/w", "state": "exited", "labels": {"herdr.space": "sp1", "herdr.tab": "tb1"}});
        let ws = workspaces(&[term("t1", "sp1", "tb1"), child, lone, gone], &Value::Null, &[]);
        assert_eq!(ids(&ws), ["hc:space:sp1", OTHER_ID]);
        let t = &ws[0].tabs[0];
        assert_eq!(t.panes.iter().map(|p| p.terminal_id.as_str()).collect::<Vec<_>>(), ["t1", "t2"], "a child sits under its parent; exited ones are gone");
        assert_eq!(t.status, AgentStatus::Blocked, "the tab shows its most urgent agent");
        assert_eq!(t.label, "tb1", "a label that is no app id is its name");
        assert_eq!(ws[1].tabs[0].label, "scratch");
        assert_eq!(ws[1].tabs[0].id, "hc:term:t9");
        // A space label with no tab: a tab of its own, named by its number.
        let ws = workspaces(&[json!({"address": "t1", "labels": {"herdr.space": "sp1"}, "argv": ["zsh"], "cwd": "/w", "state": "live"})], &Value::Null, &[]);
        assert_eq!((ws[0].tabs[0].id.as_str(), ws[0].tabs[0].label.as_str()), ("hc:tab:sp1/@t1", "1"));
    }

    #[test]
    fn document_order_and_unknown_shapes() {
        let doc = json!({"spaces": [
            {"id": "sp2", "name": "Second", "tabs": [{"id": "tbB", "root": {"id": "r", "type": "block", "content": {"terminal": "t3"}}}]},
            {"id": "sp1", "name": "First", "tabs": [{"id": "tbX", "name": "x", "root": {"id": "r", "type": "block", "content": {"terminal": "t9"}}},
                                                    {"id": "tbA", "root": {"id": "r", "type": "column", "children": [{"id": "a", "type": "block", "content": {"terminal": "t2"}}, {"id": "b", "type": "block", "content": {"terminal": "t1"}}]}}]}]});
        let terms = [term("t1", "x", "y"), term("t2", "x", "y"), term("t3", "x", "y")];
        let ws = workspaces(&terms, &doc, &[]);
        assert_eq!(ids(&ws), ["hc:space:sp2", "hc:space:sp1"], "spaces in document order");
        assert_eq!(ws[1].tabs[0].label, "2", "an unnamed tab: its place in the document");
        assert_eq!(ws[1].tabs[0].panes.iter().map(|p| p.terminal_id.as_str()).collect::<Vec<_>>(), ["t2", "t1"], "panes in tree order");
        // A shape the app does not write: labels group, the names still read.
        let odd = json!({"spaces": {"bad": 1}});
        assert!(Doc::parse(&odd).is_none());
        let ws = workspaces(&terms, &odd, &[]);
        assert_eq!(ids(&ws), ["hc:space:x"]);
        assert!(Doc::parse(&json!({"spaces": [{"id": "s", "tabs": [{"id": "t", "root": {"type": "weird"}}]}]})).is_none());
        assert!(Doc::parse(&json!([1])).is_none());
        assert!(Doc::parse(&Value::Null).is_some());
        // Deep nesting is refused, not walked forever.
        let mut deep = json!({"id": "b", "type": "block", "content": {"terminal": "t1"}});
        for _ in 0..70 {
            deep = json!({"type": "row", "children": [deep]});
        }
        assert!(Doc::parse(&json!({"spaces": [{"id": "s", "tabs": [{"id": "t", "root": deep}]}]})).is_none());
    }

    #[test]
    fn names_from_any_shape() {
        let n = Names::of(&json!({"spaces": {"sp1": {"name": "Work"}}, "tabMeta": [{"id": "tb1", "title": "Build"}], "x": {"id": "both", "label": "B"}}));
        assert_eq!(n.spaces.get("sp1").map(String::as_str), Some("Work"));
        assert_eq!(n.tabs.get("tb1").map(String::as_str), Some("Build"));
        assert!(n.spaces.get("tb1").is_none());
        assert_eq!((n.spaces.get("both").map(String::as_str), n.tabs.get("both").map(String::as_str)), (Some("B"), Some("B")));
        assert!(fresh_id(&format!("sp{}", "a".repeat(32))) && !fresh_id("sp1"));
    }

    #[test]
    fn a_read_that_panics_is_retried() {
        let running = || MACHINE_DOCS.lock().unwrap_or_else(|e| e.into_inner()).running;
        let _ = machine_docs_with(|_| panic!("a reader that fails"));
        let until = Instant::now() + Duration::from_secs(5);
        while running() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!running(), "running goes back to false after a panic");
        // The next snapshot reads again at once (no TTL wait), and its docs show.
        let got = machine_docs_with(|_| vec![("box".into(), json!({"ok": 1}))]);
        assert_eq!(got, vec![("box".to_string(), json!({"ok": 1}))]);
    }
}
