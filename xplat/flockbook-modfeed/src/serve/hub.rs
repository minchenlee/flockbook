//! The phone page's protocol (remote protocol v1, `web/src/protocol.ts`; the Mac app's `HubSession.swift`) on the
//! sidecar's own commands. A subset: hello, snapshot, watch (conversation, prompt, the agent's queue), loadEarlier,
//! pulledAck, picture, ping. Approve, answer, send, pullQueued and attach come only over `POST /api` (`post`); on the
//! socket they are refused. Notifications are not offered: those requests get a `failed` reply.
//!
//! Every read and every key goes through `cli::run`, the same code the mod's sidecar runs, so the claims, the
//! screen checks and the results are the mod's.

use crate::cli;
use herdr_core::hc::{HcClient, HcSession};
use herdr_core::{AgentStatus, Workspace};
use serde_json::{json, Value};
use std::collections::HashMap;

/// The newest rows sent per conversation (the Mac hub's window).
pub const WINDOW: usize = 100;
/// The most older rows (`loadEarlier`) one pane keeps on one connection, by count and by bytes of JSON. Past them,
/// `hasEarlier` is false: the page scrolls no further back (the Mac app has the whole log; the helper keeps a bound).
pub const MAX_OLDER: usize = 2_000;
pub const MAX_OLDER_BYTES: usize = 32 << 20;
/// The longest text `send` takes, and the longest own answer to a question (the Mac hub's limits).
pub const MAX_SEND: usize = 16_000;
pub const MAX_OTHER: usize = 2_000;

/// The requests that send keys or store something (`attach`): only `POST /api` takes them.
pub const KEYS: [&str; 5] = ["approve", "answer", "send", "pullQueued", "attach"];

/// A picture goes as it is in the log (no re-encoding, unlike the Mac hub): PNG, JPEG and WebP only, at most
/// `MAX_PICTURE` bytes (`HubImage.maxBytes`) and `MAX_SIDE`/`MAX_PIXELS`, in parts of `PICTURE_PART` bytes.
pub const MAX_PICTURE: usize = 10 << 20;
pub const PICTURE_PART: usize = 640 << 10;
pub const MAX_SIDE: i64 = 12_000;
pub const MAX_PIXELS: i64 = 50_000_000;
/// Pictures sent in `PICTURE_WINDOW_S`, on all connections together (one owner): at most this many, and bytes.
pub const MAX_PICTURES: usize = 300;
pub const MAX_PICTURE_BYTES: usize = 256 << 20;
const PICTURE_WINDOW_S: u64 = 600;

/// How often the queue of a terminal on another machine is read (each read runs the helper there).
const REMOTE_QUEUE_S: u64 = 5;

/// A text pulled back from an agent's queue that no composer has confirmed yet (`pulledAck`). Kept in this process
/// (POST and the socket are separate sessions), for every page of this owner: the next hello sends it again.
#[derive(Clone, Debug, PartialEq)]
struct Pulled {
    id: String,
    pane: String,
    text: String,
    note: Option<String>,
    interrupted: bool,
}

static PULLED: std::sync::Mutex<Vec<Pulled>> = std::sync::Mutex::new(Vec::new());
/// At most this many texts, and this many bytes of text: full, the pull clears nothing.
const PULLED_MAX: usize = 100;
const PULLED_BYTES: usize = 1 << 20;
/// The note of a kept text whose pull has not ended (the helper stopped mid-pull).
const UNFINISHED: &str = "Flockbook may not have finished clearing the agent's input box. Check the agent's terminal before you send it.";

fn pulled_line(p: &Pulled) -> Value {
    let mut v = json!({"type": "pulled", "id": p.id, "pane": p.pane, "text": p.text, "interrupted": p.interrupted});
    if let Some(n) = &p.note {
        v["note"] = json!(n);
    }
    v
}

fn pulled_store() -> std::sync::MutexGuard<'static, Vec<Pulled>> {
    PULLED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Keeps `text` for `pane` under a new id; None: full.
fn keep_pulled(pane: &str, text: &str) -> Option<String> {
    let mut all = pulled_store();
    if all.len() >= PULLED_MAX || all.iter().map(|p| p.text.len()).sum::<usize>() + text.len() > PULLED_BYTES {
        return None;
    }
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let id = format!("p{t:x}-{}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    all.push(Pulled { id: id.clone(), pane: pane.into(), text: text.into(), note: Some(UNFINISHED.into()), interrupted: false });
    Some(id)
}

/// Settings of one `serve`.
#[derive(Debug, Clone)]
pub struct Config {
    /// `--cache`: the sidecar's cache and its `answered/` claims.
    pub cache: Option<String>,
    /// `--read-only`: level `read`; every key request is refused.
    pub read_only: bool,
    /// The name the page shows.
    pub hub: String,
    /// The herdr-app home whose Flockbook settings apply (`remote_helper: never`); passed to every command.
    pub home: Option<String>,
}

/// One command of the sidecar, run in this process: its lines after `header`, without `end`.
pub fn run(argv: &[String]) -> Vec<Value> {
    let text = cli::run(argv);
    text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(|v| v["type"] != "header" && v["type"] != "end").collect()
}

fn status(s: AgentStatus) -> &'static str {
    match s {
        AgentStatus::Idle => "idle",
        AgentStatus::Working => "working",
        AgentStatus::Blocked => "blocked",
        AgentStatus::Done => "done",
        AgentStatus::Unknown => "unknown",
    }
}

/// herdr-core's workspaces as the page reads them (`HubWorkspace`).
pub fn snapshot(ws: &[Workspace]) -> Value {
    Value::Array(
        ws.iter()
            .map(|w| {
                json!({"id": w.id, "label": w.label, "tabs": w.tabs.iter().map(|t| json!({
                    "id": t.id, "label": t.label, "number": t.number, "status": status(t.status),
                    "panes": t.panes.iter().map(|p| {
                        let mut o = json!({"id": p.id, "cwd": p.cwd});
                        if let Some(l) = &p.label { o["label"] = json!(l); }
                        if let Some(a) = &p.agent {
                            let mut ag = json!({"name": a.name, "kind": a.kind, "status": status(a.status), "title": a.title});
                            if let Some(s) = &a.session_id { ag["session"] = json!(s); }
                            o["agent"] = ag;
                        }
                        o
                    }).collect::<Vec<_>>(),
                })).collect::<Vec<_>>()})
            })
            .collect(),
    )
}

/// ISO 8601 UTC with milliseconds (the Mac hub's `Hub.time`), from milliseconds since 1970.
fn time(ms: &Value) -> Option<String> {
    let ms = ms.as_i64()?;
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
}

/// A `question` object of the sidecar as the page reads it (`QuestionRequest`), with the id to answer it by.
fn question(q: &Value, id: &str) -> Value {
    let qs: Vec<Value> = q["questions"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|k| {
            let opts: Vec<Value> = k["options"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|o| {
                    let mut x = json!({"label": o["label"].as_str().unwrap_or("")});
                    if let Some(d) = o["detail"].as_str().filter(|d| !d.is_empty()) {
                        x["detail"] = json!(d);
                    }
                    if o["recommended"] == true {
                        x["recommended"] = json!(true);
                    }
                    x
                })
                .collect();
            json!({"id": k["id"].as_str().unwrap_or(""), "header": k["header"].as_str().unwrap_or(""), "text": k["text"].as_str().unwrap_or(""),
                   "multiSelect": k["multi"] == true, "options": opts})
        })
        .collect();
    let headers: Vec<Value> = qs.iter().map(|k| k["header"].clone()).collect();
    let mut out = json!({"id": id, "agent": q["agent"].as_str().unwrap_or(""), "delivery": if q["delivery"] == "message" { "message" } else { "picker" },
                         "state": q["state"].as_str().unwrap_or("pending"), "headers": headers, "questions": qs});
    if let Some(a) = q["answers"].as_array() {
        out["answers"] = Value::Array(a.clone());
    }
    out
}

/// One `row` of `feed` as a conversation item (`Item`); `sep` and `footer` rows have none (the page shows times).
pub fn item(row: &Value) -> Option<Value> {
    let id = row["id"].as_str()?;
    let mut o = match row["k"].as_str()? {
        k @ ("user" | "assistant" | "status") => json!({"id": id, "kind": k, "text": row["text"].as_str().unwrap_or("")}),
        "tools" => {
            let chips: Vec<Value> = row["chips"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|c| {
                    let calls: Vec<Value> = c["calls"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .iter()
                        .enumerate()
                        .map(|(i, x)| {
                            json!({"id": format!("{id}-{i}"), "tool": x["name"].as_str().unwrap_or(""), "summary": x["summary"].as_str().unwrap_or(""),
                                   "done": x["done"] == true, "error": x["error"] == true})
                        })
                        .collect();
                    json!({"symbol": c["symbol"].as_str().unwrap_or(""), "text": c["text"].as_str().unwrap_or(""), "calls": calls})
                })
                .collect();
            json!({"id": id, "kind": "tools", "tools": chips})
        }
        "perm" => {
            let p = &row["perm"];
            json!({"id": id, "kind": "approval", "approval": {"id": p["toolUseId"].as_str().unwrap_or(id), "tool": p["tool"].as_str().unwrap_or(""),
                   "title": p["title"].as_str().unwrap_or(""), "command": p["command"].as_str().unwrap_or(""),
                   "state": if p["state"] == "pending" { "pending" } else { "resolved" }}})
        }
        "question" => {
            let q = &row["question"];
            json!({"id": id, "kind": "question", "question": question(q, q["toolUseId"].as_str().unwrap_or(id))})
        }
        _ => return None,
    };
    if let Some(t) = time(&row["ts"]) {
        o["time"] = json!(t);
    }
    if let Some(p) = row["pics"].as_array().filter(|p| !p.is_empty()) {
        o["images"] = p.iter().map(|x| json!({"width": x["width"], "height": x["height"]})).collect();
    }
    Some(o)
}

/// The log ref a row's `pics` entry names (`rows::FeedPic`).
fn pic_ref(p: &Value) -> Option<herdr_core::log_image::Ref> {
    use herdr_core::log_image::Format;
    let format = match p["format"].as_str()? {
        "png" => Format::Png,
        "jpeg" => Format::Jpeg,
        "webp" => Format::Webp,
        "gif" => Format::Gif,
        _ => return None,
    };
    let digest = u64::from_str_radix(p["digest"].as_str()?, 16).ok()?;
    Some(herdr_core::log_image::Ref { format, width: p["width"].as_i64()?, height: p["height"].as_i64()?, offset: Some(p["offset"].as_u64()?), index: p["index"].as_u64()? as usize, digest })
}

/// A picture's mime type and size when it may be sent as it is; else why not.
pub fn offered(b: &[u8]) -> Result<(&'static str, i64, i64), &'static str> {
    use herdr_core::log_image::Format;
    let Some((f, w, h)) = herdr_core::log_image::header(b) else { return Err("The picture could not be read.") };
    let mime = match f {
        Format::Png => "image/png",
        Format::Jpeg => "image/jpeg",
        Format::Webp => "image/webp",
        Format::Gif => return Err("This picture type is not offered by this helper."),
    };
    if b.len() > MAX_PICTURE || w > MAX_SIDE || h > MAX_SIDE || w.saturating_mul(h) > MAX_PIXELS {
        return Err("The picture is too large to send.");
    }
    Ok((mime, w, h))
}

/// The pictures sent lately (when, bytes), on all connections.
static SENT_PICTURES: std::sync::Mutex<std::collections::VecDeque<(std::time::Instant, usize)>> = std::sync::Mutex::new(std::collections::VecDeque::new());

/// Room for one more picture now: under `MAX_PICTURES` and `MAX_PICTURE_BYTES` in the window.
pub fn picture_room(sent: &mut std::collections::VecDeque<(std::time::Instant, usize)>, now: std::time::Instant) -> bool {
    while sent.front().is_some_and(|(t, _)| now.duration_since(*t).as_secs() >= PICTURE_WINDOW_S) {
        sent.pop_front();
    }
    sent.len() < MAX_PICTURES && sent.iter().map(|x| x.1).sum::<usize>() < MAX_PICTURE_BYTES
}

/// The `card` line as a `prompt` push: what the agent waits for on screen now, keyed by the card's `request` (the id
/// `approve` and `answer` take back). An answered approval still drawn is no prompt (the mod shows no buttons).
pub fn prompt(pane: &str, card: Option<&Value>) -> Value {
    let mut p = json!({"type": "prompt", "pane": pane});
    let Some(c) = card else { return p };
    let request = c["request"].as_str().unwrap_or("");
    match c["kind"].as_str() {
        Some("approval") if c["answered"] != true => {
            p["approval"] = json!({"id": request, "tool": c["tool"].as_str().unwrap_or(""), "title": c["title"].as_str().unwrap_or(""),
                                   "command": c["command"].as_str().unwrap_or(""), "state": "pending"});
        }
        Some("question") => p["question"] = question(&c["question"], request),
        _ => {}
    }
    p
}

pub fn ok(re: &str) -> Value {
    json!({"type": "reply", "re": re, "ok": true})
}

pub fn fail(re: &str, code: &str, message: &str) -> Value {
    json!({"type": "reply", "re": re, "ok": false, "error": {"code": code, "message": message}})
}

/// The Mac hub's `Hub.inputProblem`: too long, or a control character other than newline and tab.
pub fn input_problem(text: &str, limit: usize) -> Option<String> {
    if text.chars().count() > limit {
        return Some(format!("Text over {limit} characters."));
    }
    text.chars()
        .find(|&c| ((c as u32) < 0x20 && c != '\n' && c != '\t') || (0x7f..=0x9f).contains(&(c as u32)))
        .map(|c| format!("Text with control character U+{:04X}.", c as u32))
}

/// A sidecar `done` (or `error`) as the reply to a key request.
fn done_reply(re: &str, lines: &[Value]) -> Value {
    if let Some(e) = lines.iter().find(|l| l["type"] == "error") {
        let code = if e["code"] == "no-terminal" || e["code"] == "not-agent" { "notFound" } else { "failed" };
        return fail(re, code, e["message"].as_str().unwrap_or("The helper could not do it."));
    }
    let Some(d) = lines.iter().find(|l| l["type"] == "done") else { return fail(re, "failed", "The helper gave no answer.") };
    let msg = d["message"].as_str();
    match d["result"].as_str().unwrap_or("") {
        "sent" => ok(re),
        "changed" => fail(re, "stale", msg.unwrap_or("The prompt changed. Nothing was sent.")),
        "answered" => fail(re, "stale", msg.unwrap_or("This prompt was answered already. Nothing was sent.")),
        "unconfirmed" => fail(re, "failed", "herdr-core did not answer in time. It may still arrive: check before sending again."),
        _ => fail(re, "failed", msg.unwrap_or("It did not go out.")),
    }
}

/// What one pane's conversation looks like on this connection (the mod's `feedstate.js`): the older rows read for
/// `loadEarlier`, then the tail `feed` keeps current.
#[derive(Default)]
struct Feed {
    cursor: Option<String>,
    rows: Vec<Value>,
    older: Vec<Value>,
    /// The log and where its tail starts (bytes); where the older rows start; the log's first row is shown.
    path: Option<String>,
    start: Option<u64>,
    older_start: Option<u64>,
    first: bool,
    /// `older` reached MAX_OLDER or MAX_OLDER_BYTES (its bytes): nothing more is read back.
    full: bool,
    older_bytes: usize,
    /// The conversation's `session` sent to the page: the log, or the pane when there is none.
    session: String,
    /// The oldest item the page has after a `loadEarlier`; None: the newest WINDOW.
    from: Option<Value>,
    sent: Option<Vec<Value>>,
    prompt: Option<Value>,
    /// The last `queued` sent (None: none yet, as an empty one); when the queue was last read.
    queued: Option<Value>,
    queue_at: Option<std::time::Instant>,
}

impl Feed {
    fn drop_older(&mut self) {
        self.from = None;
        self.older.clear();
        self.older_start = None;
        self.first = false;
        self.full = false;
        self.older_bytes = 0;
    }
    /// Where the next `feed --before` reads up to; None when the log's first row is already here.
    fn older_from(&self) -> Option<u64> {
        if self.first || self.full || self.path.is_none() || self.start.unwrap_or(0) == 0 {
            return None;
        }
        self.older_start.or(self.start)
    }
    fn items(&self) -> Vec<Value> {
        self.older.iter().chain(&self.rows).filter_map(item).collect()
    }
}

/// What to send for a conversation the page has as `sent`: `append` with the new and changed items when `sent` is
/// the start of `now` (the page merges them by id), else `replace`. None: nothing changed.
fn update(sent: Option<&Vec<Value>>, now: &[Value]) -> Option<(&'static str, Vec<Value>)> {
    let Some(sent) = sent else { return Some(("replace", now.to_vec())) };
    if sent.as_slice() == now {
        return None;
    }
    if sent.len() <= now.len() && sent.iter().zip(now).all(|(a, b)| a["id"] == b["id"]) {
        let changed = now.iter().enumerate().filter(|(i, n)| sent.get(*i) != Some(n)).map(|(_, n)| n.clone()).collect();
        return Some(("append", changed));
    }
    Some(("replace", now.to_vec()))
}

/// One page connection (or one POST): its watched panes and what it was sent last.
pub struct Session {
    cfg: Config,
    pub welcomed: bool,
    watched: Vec<String>,
    feeds: HashMap<String, Feed>,
    last_snapshot: Option<Value>,
    workspaces: Vec<Workspace>,
}

/// herdr-core's workspaces now: spaces and tabs as the Mac app shows them, other machines' too (`layout`). Empty
/// when it does not answer. `home`: the herdr-app home whose settings say whether other machines are read.
pub fn workspaces(home: Option<&str>) -> Vec<herdr_core::Workspace> {
    let session = HcSession::resolve_env();
    if !session.is_available() {
        return vec![];
    }
    let never = crate::config::load(crate::config::dir(home).as_deref()).remote_never;
    super::layout::snapshot(&HcClient::new(session, 8, 22), never).unwrap_or_default()
}

fn now_s() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Session {
    pub fn new(cfg: Config) -> Session {
        Session { cfg, welcomed: false, watched: vec![], feeds: HashMap::new(), last_snapshot: None, workspaces: vec![] }
    }

    fn level(&self) -> &'static str {
        if self.cfg.read_only {
            "read"
        } else {
            "input"
        }
    }

    fn base(&self, cmd: &str, pane: &str) -> Vec<String> {
        let mut a = vec![cmd.to_string(), "--terminal".into(), pane.to_string()];
        if let Some(c) = &self.cfg.cache {
            a.push("--cache".into());
            a.push(c.clone());
        }
        // Last, as the mod passes it: the last --home counts.
        if let Some(h) = &self.cfg.home {
            a.extend(["--home".into(), h.clone()]);
        }
        a
    }

    /// herdr-core's workspaces now; empty when it does not answer.
    fn load(&mut self) -> Value {
        self.workspaces = workspaces(self.cfg.home.as_deref());
        snapshot(&self.workspaces)
    }

    fn pane(&self, id: &str) -> Option<&herdr_core::Pane> {
        self.workspaces.iter().flat_map(|w| &w.tabs).flat_map(|t| &t.panes).find(|p| p.id == id || p.terminal_id == id)
    }

    /// A socket message. Key requests are refused here (`post` takes them).
    pub fn receive(&mut self, data: &str) -> Vec<Value> {
        let (m, id) = match parse(data) {
            Ok(x) => x,
            Err(r) => return vec![r],
        };
        let ty = m["type"].as_str().unwrap_or("");
        if ty == "hello" {
            self.welcomed = true;
            let snap = self.load();
            self.last_snapshot = Some(snap.clone());
            let session = std::env::var("HC_SESSION").ok().filter(|s| !s.is_empty());
            let mut w = json!({"type": "welcome", "hub": self.cfg.hub, "hubVersion": crate::protocol::SIDECAR_VERSION, "level": self.level()});
            if let Some(s) = session {
                w["session"] = json!(s);
            }
            // One owner, no device ids: on when the helper keeps any subscription (the page also checks the key).
            if let Some(c) = super::push::center().as_ref() {
                w["pushKey"] = json!(c.public_key());
                w["pushOn"] = json!(!c.store.all.is_empty());
            }
            // Pulled texts no composer confirmed yet (the connection dropped before), with their warning.
            let mut out = vec![w, json!({"type": "snapshot", "workspaces": snap})];
            out.extend(pulled_store().iter().map(pulled_line));
            out.push(ok(&id));
            return out;
        }
        if !self.welcomed {
            return vec![fail(&id, "unauthorized", "Send hello first.")];
        }
        match ty {
            "ping" => vec![json!({"type": "pong"}), ok(&id)],
            "watch" => {
                let pane = m["pane"].as_str().unwrap_or("").to_string();
                self.load();
                if self.pane(&pane).is_none() {
                    return vec![fail(&id, "notFound", &format!("No pane {pane}."))];
                }
                if !self.watched.contains(&pane) {
                    self.watched.push(pane.clone());
                }
                self.feeds.insert(pane.clone(), Feed::default());
                let mut out = self.pushes(&pane);
                out.push(ok(&id));
                out
            }
            "unwatch" => {
                let pane = m["pane"].as_str().unwrap_or("");
                self.watched.retain(|p| p != pane);
                self.feeds.remove(pane);
                vec![ok(&id)]
            }
            t if KEYS.contains(&t) => vec![fail(&id, "badRequest", "Keys go with POST /api on this helper.")],
            "pair" => vec![fail(&id, "badRequest", "This helper needs no pairing: Tailscale says who you are.")],
            "loadEarlier" => {
                let (Some(session), Some(before)) = (m["session"].as_str(), m["before"].as_str()) else {
                    return vec![fail(&id, "badRequest", "loadEarlier needs session and before.")];
                };
                self.load_earlier(session, before, &id)
            }
            "pulledAck" => {
                let Some(p) = m["pulled"].as_str() else { return vec![fail(&id, "badRequest", "pulledAck needs pulled.")] };
                pulled_store().retain(|x| x.id != p);
                vec![ok(&id)]
            }
            "picture" => {
                let (Some(pane), Some(item), Some(index)) = (m["pane"].as_str(), m["item"].as_str(), m["index"].as_u64()) else {
                    return vec![fail(&id, "badRequest", "picture needs pane, item and index.")];
                };
                self.picture(&id, pane, item, index as usize, m["full"] == true)
            }
            // Any level, --read-only too: a notification carries no conversation text.
            "pushSubscribe" => {
                let str_of = |k: &str| m[k].as_str().unwrap_or("").to_string();
                let sub = super::push::Sub { endpoint: str_of("endpoint"), p256dh: str_of("p256dh"), auth: str_of("auth"), added: now_s() };
                if !sub.valid() {
                    return vec![fail(&id, "badRequest", "Not a push service this hub sends to.")];
                }
                if super::push::center().as_mut().is_some_and(|c| c.store.add(sub)) {
                    vec![ok(&id)]
                } else {
                    vec![fail(&id, "failed", "Notifications are off on this hub.")]
                }
            }
            "pushUnsubscribe" => {
                if let (Some(c), Some(e)) = (super::push::center().as_mut(), m["endpoint"].as_str()) {
                    c.store.remove(e);
                }
                vec![ok(&id)]
            }
            _ => vec![fail(&id, "failed", "Not offered by this helper.")],
        }
    }

    /// `POST /api`: one key request; the reply (and nothing else).
    pub fn post(&mut self, data: &str) -> Vec<Value> {
        let (m, id) = match parse(data) {
            Ok(x) => x,
            Err(r) => return vec![r],
        };
        let ty = m["type"].as_str().unwrap_or("");
        if !KEYS.contains(&ty) {
            return vec![fail(&id, "badRequest", "POST /api takes approve, answer, send, pullQueued and attach.")];
        }
        if self.cfg.read_only {
            return vec![fail(&id, "readOnly", "This helper was started with --read-only.")];
        }
        if ty == "attach" {
            return self.attach(&m, &id);
        }
        let pane = m["pane"].as_str().unwrap_or("").to_string();
        self.load();
        let Some(p) = self.pane(&pane).cloned() else { return vec![fail(&id, "notFound", &format!("No pane {pane}."))] };
        if p.agent.is_none() {
            return vec![fail(&id, "readOnly", "Remote input goes to agents only.")];
        }
        // Typed values to the sidecar's own functions (`cli::keys`): nothing from the page is parsed as an option.
        let cache = self.cfg.cache.as_deref().map(std::path::Path::new);
        if ty == "pullQueued" {
            return pull_queued(&id, &pane, &p.terminal_id, cache);
        }
        let mut sent_names: Vec<String> = vec![];
        let keys = match ty {
            "approve" => {
                let (Some(prompt), Some(allow)) = (m["prompt"].as_str(), m["allow"].as_bool()) else {
                    return vec![fail(&id, "badRequest", "approve needs prompt and allow.")];
                };
                cli::Keys::Approve { request: prompt.into(), allow }
            }
            "answer" => {
                let (Some(q), Some(answers)) = (m["question"].as_str(), m["answers"].as_array()) else {
                    return vec![fail(&id, "badRequest", "answer needs question and answers.")];
                };
                for a in answers {
                    if let Some(why) = a["other"].as_str().and_then(|o| input_problem(o, MAX_OTHER)) {
                        return vec![fail(&id, "badRequest", &why)];
                    }
                }
                let Some(answers) = crate::cards::parse_answers(&Value::Array(answers.clone()).to_string()) else {
                    return vec![fail(&id, "badRequest", "answers: one {picks, other} per question.")];
                };
                cli::Keys::Answer { request: q.into(), answers }
            }
            _ => {
                let text = m["text"].as_str().unwrap_or("");
                if let Some(why) = input_problem(text, MAX_SEND) {
                    return vec![fail(&id, "badRequest", &why)];
                }
                let names: Vec<String> = match m["attachments"].as_array() {
                    None => vec![],
                    Some(a) => match a.iter().map(|n| n.as_str().map(String::from)).collect::<Option<Vec<_>>>() {
                        Some(n) => n,
                        None => return vec![fail(&id, "badRequest", "attachments: upload names.")],
                    },
                };
                if !names.is_empty() && !matches!(crate::remote::split(&p.terminal_id), Ok(None)) {
                    return vec![fail(&id, "badRequest", "Images go to this machine's terminals only. Nothing was sent.")];
                }
                let files = match super::upload::files(&names) {
                    Ok(f) => f,
                    Err(why) => return vec![fail(&id, "badRequest", &why)],
                };
                if text.is_empty() && files.is_empty() {
                    return vec![fail(&id, "badRequest", "Nothing to send.")];
                }
                // The text, then each image's path, as the Mac hub types them (the agent reads the files).
                let quoted = files.iter().map(|f| herdr_core::hc::client::shell_quoted_if_needed(&f.to_string_lossy()));
                let message = std::iter::once(text.to_string()).chain(quoted).filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ");
                sent_names = names;
                // The agent the page showed, and its log (the conversation's `session`, when it is a log): another
                // agent or another conversation in this pane now gets nothing (the mod's --expect-kind, --expect-path).
                let Some(kind) = m["agent"].as_str().filter(|k| !k.is_empty()) else {
                    return vec![fail(&id, "badRequest", "send needs agent: the agent kind the page shows.")];
                };
                let path = m["session"].as_str().filter(|s| !s.is_empty() && *s != pane && *s != p.terminal_id).map(String::from);
                cli::Keys::Send { text: message, expect_kind: Some(kind.into()), expect_path: path }
            }
        };
        let line = cli::keys(&p.terminal_id, cache, self.cfg.home.as_deref(), keys);
        if line["result"] == "sent" {
            super::upload::sent(&sent_names);
        }
        vec![done_reply(&id, std::slice::from_ref(&line))]
    }

    /// Pushes since the last call: a changed snapshot, and new rows and prompts of the watched panes.
    pub fn tick(&mut self) -> Vec<Value> {
        if !self.welcomed {
            return vec![];
        }
        let snap = self.load();
        let mut out = vec![];
        if self.last_snapshot.as_ref() != Some(&snap) {
            self.last_snapshot = Some(snap.clone());
            out.push(json!({"type": "snapshot", "workspaces": snap}));
        }
        for pane in self.watched.clone() {
            out.extend(self.pushes(&pane));
        }
        out
    }

    fn pushes(&mut self, pane: &str) -> Vec<Value> {
        let Some(p) = self.pane(pane).cloned() else { return vec![] };
        let mut out = vec![];
        let terminal = p.terminal_id.clone();
        let mut argv = self.feed_argv(&terminal);
        let cursor = self.feeds.get(pane).and_then(|f| f.cursor.clone());
        if let Some(c) = &cursor {
            argv.extend(["--cursor".into(), c.clone()]);
        }
        let lines = run(&argv);
        let feed = self.feeds.entry(pane.to_string()).or_default();
        for l in &lines {
            match l["type"].as_str() {
                Some("log") => {
                    feed.cursor = l["cursor"].as_str().map(String::from);
                    let path = l["path"].as_str().map(String::from);
                    let start = l["start"].as_u64();
                    // Another log, or its tail starts elsewhere: the older rows no longer join it.
                    if path != feed.path || start != feed.start {
                        if path != feed.path {
                            feed.rows.clear();
                        }
                        feed.drop_older();
                    }
                    feed.session = path.clone().unwrap_or_else(|| pane.to_string());
                    (feed.path, feed.start) = (path, start);
                }
                Some("splice") => {
                    let at = l["at"].as_u64().unwrap_or(0) as usize;
                    feed.rows.truncate(at.min(feed.rows.len()));
                }
                Some("row") => feed.rows.push(l.clone()),
                _ => {}
            }
        }
        if feed.session.is_empty() {
            feed.session = pane.to_string();
        }
        let items = feed.items();
        let lo = feed.from.as_ref().and_then(|f| items.iter().position(|i| &i["id"] == f)).unwrap_or(items.len().saturating_sub(WINDOW));
        let tail = &items[lo..];
        if let Some((mode, send)) = update(feed.sent.as_ref(), tail) {
            let earlier = lo > 0 || feed.older_from().is_some();
            out.push(json!({"type": "conversation", "pane": pane, "session": feed.session, "mode": mode, "items": send, "hasEarlier": earlier}));
            feed.sent = Some(tail.to_vec());
        }
        // The screen is read only when there may be a prompt: the agent is blocked, or the log ends with a question.
        let blocked = p.agent.as_ref().is_some_and(|a| a.status == AgentStatus::Blocked);
        let asks = feed.rows.last().is_some_and(|r| r["k"] == "question" && r["question"]["state"] == "pending");
        let card = if blocked || asks { run(&self.base("card", &terminal)).into_iter().find(|l| l["type"] == "card") } else { None };
        let pr = prompt(pane, card.as_ref());
        let feed = self.feeds.entry(pane.to_string()).or_default();
        if feed.prompt.as_ref() != Some(&pr) {
            feed.prompt = Some(pr.clone());
            out.push(pr);
        }
        // The agent's own queue: on watch only when there is one; then each change, an empty one clearing it. Read
        // only while the agent works; on another machine at most every REMOTE_QUEUE_S.
        let reads = p.agent.as_ref().is_some_and(|a| a.status == AgentStatus::Working && crate::queue::reads(&a.kind));
        let remote = matches!(crate::remote::split(&terminal), Ok(Some(_)));
        let due = !remote || feed.queue_at.is_none_or(|t| t.elapsed().as_secs() >= REMOTE_QUEUE_S);
        if !reads || due {
            let mut q = if reads {
                feed.queue_at = Some(std::time::Instant::now());
                run(&self.base("queue", &terminal)).into_iter().find(|l| l["type"] == "queued").unwrap_or_else(|| crate::queue::line(None))
            } else {
                crate::queue::line(None)
            };
            q["pane"] = json!(pane);
            let feed = self.feeds.entry(pane.to_string()).or_default();
            let was = feed.queued.clone().unwrap_or_else(|| {
                let mut e = crate::queue::line(None);
                e["pane"] = json!(pane);
                e
            });
            if q != was {
                out.push(q.clone());
            }
            feed.queued = Some(q);
        }
        out
    }
}

impl Session {
    /// `loadEarlier`: up to WINDOW items before `before` in the watched pane that shows `session` (the Mac hub's
    /// `loadEarlier`). The log is read further back only when fewer than WINDOW are here. Only this connection's own
    /// log path and offset go to `feed --before`; the page's values are only compared.
    /// `attach`: one part of an image for the next `send` (`upload::attach`).
    fn attach(&self, m: &Value, id: &str) -> Vec<Value> {
        let Some(home) = &self.cfg.home else { return vec![fail(id, "failed", "This helper takes no images: it has no herdr-app home.")] };
        let (Some(name), Some(part), Some(parts), Some(data)) = (m["upload"].as_str(), m["part"].as_u64(), m["parts"].as_u64(), m["data"].as_str()) else {
            return vec![fail(id, "badRequest", "attach needs upload, part, parts and data.")];
        };
        let dir = super::upload::folder(std::path::Path::new(home));
        match super::upload::attach(&dir, name, part as usize, parts as usize, data, std::time::Instant::now(), std::time::SystemTime::now()) {
            Ok(Some(attached)) => vec![attached, ok(id)],
            Ok(None) => vec![ok(id)],
            Err((code, why)) => vec![fail(id, code, &why)],
        }
    }

    /// `feed` for `terminal`: on this machine its rows say where their pictures are (`--pictures`). A terminal on
    /// another machine shows none: its log is there.
    fn feed_argv(&self, terminal: &str) -> Vec<String> {
        let mut a = self.base("feed", terminal);
        if matches!(crate::remote::split(terminal), Ok(None)) {
            a.push("--pictures".into());
        }
        a
    }

    /// One picture of a watched pane's conversation (remote-protocol.md 3.4), read back from the log (`log_image::read`
    /// checks its fingerprint) and sent as it is, in `picturePart`s, then ok.
    fn picture(&mut self, re: &str, pane: &str, item: &str, index: usize, full: bool) -> Vec<Value> {
        let Some(feed) = self.feeds.get(pane).filter(|_| self.watched.iter().any(|p| p == pane)) else {
            return vec![fail(re, "badRequest", "Watch the pane first.")];
        };
        let Some(row) = feed.older.iter().chain(&feed.rows).find(|r| r["id"] == item && self::item(r).is_some()) else {
            return vec![fail(re, "notFound", &format!("No item {item}."))];
        };
        let Some(p) = row["pics"].as_array().and_then(|p| p.get(index)) else {
            return vec![fail(re, "notFound", &format!("No picture {index} in item {item}."))];
        };
        let gone = || vec![fail(re, "notFound", "The picture is no longer in the log.")];
        let (Some(r), Some(path)) = (pic_ref(p), feed.path.clone()) else { return gone() };
        let now = std::time::Instant::now();
        if !picture_room(&mut SENT_PICTURES.lock().unwrap_or_else(|e| e.into_inner()), now) {
            return vec![fail(re, "rateLimited", "Too many pictures at once. Try again soon.")];
        }
        let Some(bytes) = herdr_core::log_image::read(&r, std::path::Path::new(&path)) else { return gone() };
        let (mime, width, height) = match offered(&bytes) {
            Ok(x) => x,
            Err(why) => return vec![fail(re, "failed", why)],
        };
        SENT_PICTURES.lock().unwrap_or_else(|e| e.into_inner()).push_back((now, bytes.len()));
        use base64::Engine;
        let chunks: Vec<&[u8]> = bytes.chunks(PICTURE_PART).collect();
        let mut out: Vec<Value> = chunks
            .iter()
            .enumerate()
            .map(|(i, d)| {
                json!({"type": "picturePart", "re": re, "pane": pane, "item": item, "index": index, "full": full, "part": i, "parts": chunks.len(),
                       "mime": mime, "width": width, "height": height, "data": base64::engine::general_purpose::STANDARD.encode(d)})
            })
            .collect();
        out.push(ok(re));
        out
    }

    fn load_earlier(&mut self, session: &str, before: &str, id: &str) -> Vec<Value> {
        let Some(pane) = self.watched.iter().find(|p| self.feeds.get(*p).is_some_and(|f| f.session == session)).cloned() else {
            return vec![fail(id, "notFound", &format!("No watched pane shows session {session}."))];
        };
        let Some(terminal) = self.pane(&pane).map(|p| p.terminal_id.clone()) else { return vec![fail(id, "notFound", &format!("No pane {pane}."))] };
        let feed = &self.feeds[&pane];
        let at = |items: &[Value]| items.iter().position(|i| i["id"] == before);
        let Some(idx) = at(&feed.items()) else { return vec![fail(id, "notFound", &format!("No item {before}."))] };
        if idx < WINDOW {
            if let (Some(from), Some(path)) = (feed.older_from(), feed.path.clone()) {
                let mut argv = self.feed_argv(&terminal);
                argv.extend(["--before".into(), from.to_string(), "--path".into(), path.clone()]);
                let lines = run(&argv);
                let head = lines.iter().find(|l| l["type"] == "older");
                let Some(head) = head.filter(|h| h["path"].as_str() == Some(path.as_str())) else {
                    return vec![fail(id, "failed", "Could not load earlier messages. Try again.")];
                };
                let start = head["start"].as_u64();
                let feed = self.feeds.get_mut(&pane).expect("watched");
                let have: std::collections::HashSet<Value> = feed.older.iter().chain(&feed.rows).map(|r| r["id"].clone()).collect();
                let mut add: Vec<Value> = lines.iter().filter(|l| l["type"] == "row" && !have.contains(&l["id"])).cloned().collect();
                // Within the bound: the newest of them, and then no more reads.
                let mut keep = 0;
                for r in add.iter().rev() {
                    let b = r.to_string().len();
                    if feed.older.len() + keep >= MAX_OLDER || feed.older_bytes + b > MAX_OLDER_BYTES {
                        feed.full = true;
                        break;
                    }
                    feed.older_bytes += b;
                    keep += 1;
                }
                add.drain(..add.len() - keep);
                feed.older.splice(0..0, add);
                feed.older_start = start;
                feed.first = start == Some(0);
            }
        }
        let feed = self.feeds.get_mut(&pane).expect("watched");
        let items = feed.items();
        let idx = at(&items).unwrap_or(idx.min(items.len()));
        let lo = idx.saturating_sub(WINDOW);
        let older = items[lo..idx].to_vec();
        if let Some(sent) = feed.sent.as_mut().filter(|s| s.first().is_some_and(|f| f["id"] == before)) {
            sent.splice(0..0, older.iter().cloned());
            feed.from = sent.first().map(|i| i["id"].clone());
        }
        let earlier = lo > 0 || feed.older_from().is_some();
        vec![json!({"type": "conversation", "pane": pane, "session": session, "mode": "prepend", "items": older, "hasEarlier": earlier}), ok(id)]
    }
}

/// `pullQueued`: the agent's queue back to the page's composer (`cli::pull`). The text is kept (`PULLED`) before the
/// agent's box is cleared, until the composer confirms it (`pulledAck`).
fn pull_queued(re: &str, pane: &str, terminal: &str, cache: Option<&std::path::Path>) -> Vec<Value> {
    let mut kept: Option<String> = None;
    let mut keep = |text: &str| {
        kept = keep_pulled(pane, text);
        kept.is_some()
    };
    let r = match cli::pull(terminal, cache, &mut keep) {
        Ok(r) => r,
        Err((code, message)) => return vec![done_reply(re, &[json!({"type": "error", "code": code, "message": message})])],
    };
    use crate::queue::Outcome;
    let (note, interrupted) = match &r {
        Outcome::Partial(_, why) => (Some(why.clone()), false),
        Outcome::Interrupted(_) => (None, true),
        _ => (None, false),
    };
    if let Some(id) = &kept {
        if let Some(p) = pulled_store().iter_mut().find(|p| &p.id == id) {
            (p.note, p.interrupted) = (note.clone(), interrupted);
        }
    }
    match r {
        Outcome::Pulled(text) | Outcome::Interrupted(text) | Outcome::Partial(text, _) => {
            let mut l = pulled_line(&Pulled { id: String::new(), pane: pane.into(), text, note, interrupted });
            match kept {
                Some(id) => l["id"] = json!(id),
                None => _ = l.as_object_mut().map(|o| o.remove("id")),
            }
            vec![l, ok(re)]
        }
        Outcome::Refused(why) | Outcome::Failed(why) => vec![fail(re, "failed", &why)],
    }
}

/// A client message: protocol 1, an id. Errors are the reply to send.
/// A `Value`, not a `Map`: a missing field indexes as null (a `Map` index panics).
fn parse(data: &str) -> Result<(Value, String), Value> {
    let v: Value = serde_json::from_str(data).map_err(|_| fail("", "badRequest", "Not a protocol 1 client message."))?;
    let id = v["id"].as_str().unwrap_or("").to_string();
    let Value::Object(m) = v else { return Err(fail("", "badRequest", "Not a protocol 1 client message.")) };
    if m.get("v").and_then(Value::as_i64) != Some(1) {
        return Err(fail(&id, "unsupportedVersion", "This helper speaks protocol 1."));
    }
    if id.is_empty() || m.get("type").and_then(Value::as_str).is_none() {
        return Err(fail(&id, "badRequest", "Not a protocol 1 client message."));
    }
    Ok((Value::Object(m), id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, len: usize) -> Vec<u8> {
        let mut b = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        b.extend(b"IHDR");
        b.extend(w.to_be_bytes());
        b.extend(h.to_be_bytes());
        b.resize(len.max(b.len()), 0);
        b
    }

    #[test]
    fn pictures_offered_as_they_are() {
        assert_eq!(offered(&png(10, 20, 100)), Ok(("image/png", 10, 20)));
        let jpeg = [0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x30, 0x00, 0x40, 0x03, 0, 0, 0, 0, 0];
        assert_eq!(offered(&jpeg), Ok(("image/jpeg", 64, 48)));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        webp.extend([0u8; 8]);
        webp.extend([9, 0, 0, 4, 0, 0]); // 10 x 5
        assert_eq!(offered(&webp), Ok(("image/webp", 10, 5)));
        assert!(offered(b"GIF89a\x0a\x00\x14\x00").unwrap_err().contains("not offered"));
        assert!(offered(b"<svg/>").is_err());
        assert!(offered(&png(12_001, 1, 100)).unwrap_err().contains("too large"));
        assert!(offered(&png(8_000, 8_000, 100)).unwrap_err().contains("too large"), "64 M pixels");
        assert!(offered(&png(10, 10, MAX_PICTURE)).is_ok());
        assert!(offered(&png(10, 10, MAX_PICTURE + 1)).unwrap_err().contains("too large"));
    }

    #[test]
    fn pictures_have_a_window() {
        let t0 = std::time::Instant::now();
        let mut sent = std::collections::VecDeque::new();
        for _ in 0..MAX_PICTURES {
            assert!(picture_room(&mut sent, t0));
            sent.push_back((t0, 1));
        }
        assert!(!picture_room(&mut sent, t0), "count");
        assert!(picture_room(&mut sent, t0 + std::time::Duration::from_secs(PICTURE_WINDOW_S)), "the window moved on");
        assert!(sent.is_empty());
        sent.push_back((t0, MAX_PICTURE_BYTES));
        assert!(!picture_room(&mut sent, t0), "bytes");
    }

    #[test]
    fn rows_say_where_pictures_are() {
        let u = item(&json!({"id": "u1", "k": "user", "text": "", "type": "row",
            "pics": [{"format": "png", "width": 3, "height": 4, "offset": 0, "index": 0, "digest": "00ff00ff00ff00ff"}]})).unwrap();
        assert_eq!(u["images"], json!([{"width": 3, "height": 4}]));
        let r = pic_ref(&json!({"format": "png", "width": 3, "height": 4, "offset": 9, "index": 2, "digest": "00ff00ff00ff00ff"})).unwrap();
        assert_eq!((r.offset, r.index, r.digest), (Some(9), 2, 0x00ff00ff00ff00ff));
        assert!(pic_ref(&json!({"format": "bmp", "width": 3, "height": 4, "offset": 9, "index": 2, "digest": "00"})).is_none());
        assert!(item(&json!({"id": "u2", "k": "user", "text": "", "type": "row"})).unwrap().get("images").is_none());
    }

    #[test]
    fn rows_to_items() {
        let u = item(&json!({"id": "u1", "k": "user", "text": "hi", "ts": 1793670000000_i64, "type": "row"})).unwrap();
        assert_eq!(u, json!({"id": "u1", "kind": "user", "text": "hi", "time": "2026-11-03T01:40:00.000Z"}));
        assert!(item(&json!({"id": "f1", "k": "footer", "text": "x"})).is_none());
        assert!(item(&json!({"id": "s1", "k": "sep", "text": "x"})).is_none());
        let t = item(&json!({"id": "a1-t3", "k": "tools", "chips": [{"symbol": "terminal", "text": "Ran a command",
            "calls": [{"name": "Bash", "summary": "ls", "done": true, "error": false, "output": "ok"}]}]})).unwrap();
        assert_eq!(t["tools"][0]["calls"][0], json!({"id": "a1-t3-0", "tool": "Bash", "summary": "ls", "done": true, "error": false}));
        let p = item(&json!({"id": "p1", "k": "perm", "perm": {"toolUseId": "toolu_1", "tool": "Bash", "title": "Bash command", "command": "rm", "state": "pending"}})).unwrap();
        assert_eq!(p["approval"], json!({"id": "toolu_1", "tool": "Bash", "title": "Bash command", "command": "rm", "state": "pending"}));
    }

    #[test]
    fn cards_to_prompts() {
        assert_eq!(prompt("hc:t1", None), json!({"type": "prompt", "pane": "hc:t1"}));
        let a = json!({"type": "card", "kind": "approval", "request": "r1", "tool": "Bash", "title": "Bash command", "command": "rm -rf build"});
        assert_eq!(prompt("hc:t1", Some(&a))["approval"]["id"], "r1");
        let mut done = a.clone();
        done["answered"] = json!(true);
        assert!(prompt("hc:t1", Some(&done)).get("approval").is_none(), "an answered prompt shows no buttons");
        let q = json!({"type": "card", "kind": "question", "request": "screen-1", "question": {"agent": "claude", "delivery": "picker", "state": "pending",
            "toolUseId": "screen-1", "questions": [{"id": "Pick?", "header": "Color", "text": "Pick?", "multi": false,
            "options": [{"label": "Red", "detail": "warm", "recommended": true}, {"label": "Blue", "detail": "", "recommended": false}]}]}});
        let pq = prompt("hc:t1", Some(&q));
        assert_eq!(pq["question"]["id"], "screen-1");
        assert_eq!(pq["question"]["headers"], json!(["Color"]));
        assert_eq!(pq["question"]["questions"][0]["options"], json!([{"label": "Red", "detail": "warm", "recommended": true}, {"label": "Blue"}]));
    }

    #[test]
    fn updates() {
        let a = |id: &str, t: &str| json!({"id": id, "text": t});
        assert_eq!(update(None, &[a("1", "x")]), Some(("replace", vec![a("1", "x")])));
        assert_eq!(update(Some(&vec![a("1", "x")]), &[a("1", "x")]), None);
        // A new item and a changed last one: only those.
        assert_eq!(update(Some(&vec![a("1", "x"), a("2", "y")]), &[a("1", "x"), a("2", "y2"), a("3", "z")]), Some(("append", vec![a("2", "y2"), a("3", "z")])));
        // An item gone or another first item: replace.
        assert_eq!(update(Some(&vec![a("1", "x"), a("2", "y")]), &[a("1", "x")]).unwrap().0, "replace");
        assert_eq!(update(Some(&vec![a("1", "x")]), &[a("0", "w"), a("1", "x")]).unwrap().0, "replace");
    }

    #[test]
    fn replies() {
        assert_eq!(done_reply("c1", &[json!({"type": "done", "result": "sent"})]), ok("c1"));
        assert_eq!(done_reply("c1", &[json!({"type": "done", "result": "changed"})])["error"]["code"], "stale");
        assert_eq!(done_reply("c1", &[json!({"type": "error", "code": "no-terminal", "message": "x"})])["error"]["code"], "notFound");
        assert_eq!(input_problem("a\u{1b}[2J", 100).as_deref(), Some("Text with control character U+001B."));
        assert!(input_problem("line\n\tok", 100).is_none());
        assert!(input_problem(&"x".repeat(11), 10).is_some());
    }

    #[test]
    fn socket_refuses_keys() {
        // hello loads a snapshot: never from the herdr-core this machine runs.
        std::env::set_var("HC_SOCKET", "/nonexistent/flockbook-test/hc.sock");
        std::env::remove_var("HERDR_DESK_HC_SOCKET");
        let mut s = Session::new(Config { cache: None, read_only: false, hub: "test".into(), home: None });
        s.welcomed = true;
        for t in KEYS {
            let r = s.receive(&json!({"v": 1, "id": "c1", "type": t, "pane": "hc:t1", "prompt": "x", "allow": true, "text": "x"}).to_string());
            assert_eq!(r, vec![fail("c1", "badRequest", "Keys go with POST /api on this helper.")]);
        }
        assert_eq!(s.receive(r#"{"v":2,"id":"c2","type":"ping"}"#)[0]["error"]["code"], "unsupportedVersion");
        // Missing fields are null, never a panic.
        assert_eq!(s.receive(r#"{"v":1,"id":"c5","type":"unwatch"}"#), vec![ok("c5")]);
        assert_eq!(s.post(r#"{"v":1,"id":"c6","type":"send"}"#)[0]["re"], "c6");
        let mut ro = Session::new(Config { cache: None, read_only: true, hub: "test".into(), home: None });
        assert_eq!(ro.post(r#"{"v":1,"id":"c3","type":"send","pane":"hc:t1","text":"x"}"#)[0]["error"]["code"], "readOnly");
        assert_eq!(ro.post(r#"{"v":1,"id":"c4","type":"watch","pane":"hc:t1"}"#)[0]["error"]["code"], "badRequest");
    }

    #[test]
    fn phones_subscribe_for_notifications() {
        use p256::elliptic_curve::sec1::ToEncodedPoint;
        std::env::set_var("HC_SOCKET", "/nonexistent/flockbook-test/hc.sock");
        std::env::remove_var("HERDR_DESK_HC_SOCKET");
        let dir = tempfile::tempdir().unwrap();
        let ua = p256::SecretKey::random(&mut rand_core::OsRng);
        let p256dh = super::super::push::b64url(ua.public_key().to_encoded_point(false).as_bytes());
        let auth = super::super::push::b64url(&[7; 16]);
        let fcm = "https://fcm.googleapis.com/fcm/send/x";
        let sub = |id: &str, e: &str| json!({"v": 1, "id": id, "type": "pushSubscribe", "endpoint": e, "p256dh": p256dh, "auth": auth}).to_string();
        let welcome = |s: &mut Session| s.receive(r#"{"v":1,"id":"h","type":"hello"}"#)[0].clone();
        let mut s = Session::new(Config { cache: None, read_only: true, hub: "test".into(), home: None });
        assert_eq!(s.receive(&sub("p0", fcm))[0]["error"]["code"], "unauthorized");
        welcome(&mut s);
        assert_eq!(s.receive(&sub("p1", fcm))[0]["error"]["code"], "failed", "push off: nothing kept");
        let key = p256::ecdsa::SigningKey::random(&mut rand_core::OsRng);
        let store = super::super::push::Store::open(dir.path()).unwrap();
        *super::super::push::center() = Some(super::super::push::Center::new(key, store, Box::new(|_, _, _| 201)));
        let w = welcome(&mut s);
        assert_eq!(w["pushOn"], false);
        assert_eq!(w["pushKey"].as_str().map(|k| super::super::push::unb64url(k).unwrap().len()), Some(65));
        assert_eq!(s.receive(&sub("p2", "https://evil.example/x"))[0]["error"]["code"], "badRequest");
        assert_eq!(s.receive(&sub("p3", fcm)), vec![ok("p3")], "read-only may subscribe");
        assert_eq!(welcome(&mut s)["pushOn"], true);
        assert_eq!(s.receive(&json!({"v": 1, "id": "p4", "type": "pushUnsubscribe", "endpoint": fcm}).to_string()), vec![ok("p4")]);
        assert_eq!(welcome(&mut s)["pushOn"], false);
        *super::super::push::center() = None;
    }

}
