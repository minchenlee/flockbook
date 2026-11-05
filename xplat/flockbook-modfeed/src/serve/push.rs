//! Web Push for the phone page (the Mac hub's `WebPush`, `WebPushSender`, `PushStore`, `PushThrottle`, `PushCenter`).
//! - Messages are encrypted for the phone (RFC 8291, "aes128gcm" of RFC 8188); each request carries VAPID (RFC 8292).
//!   The push service sees the size and time, never the text.
//! - A message is {title, body, tag, pane}: the agent's title, its status and where it runs. No conversation text.
//! - `serve` is a service (launchd, systemd), so it pushes from its own loop, with no page open: a pane that turns
//!   blocked (needs you) or done. The first snapshot only records. Throttled as on the Mac.
//! - Only https endpoints of the known push services are ever contacted; redirects are not followed (curl, as the
//!   update check). A 404 or 410 deletes the subscription.
//! - The key (`vapid.key`) and the subscriptions (`subscriptions.json`) are files only the owner can read (mode 600)
//!   in `<herdr-app home>/flockbook/serve`. A key file that does not hold a key is never overwritten: push stays off.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use base64::Engine;
use herdr_core::{AgentStatus, Workspace};
use hkdf::Hkdf;
use p256::ecdsa::signature::Signer;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use rand_core::{OsRng, RngCore};
use serde_json::{json, Value};
use sha2::Sha256;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

pub fn b64url(d: &[u8]) -> String {
    B64.encode(d)
}

/// base64url, with or without padding.
pub fn unb64url(s: &str) -> Option<Vec<u8>> {
    B64.decode(s.trim().trim_end_matches('=')).ok()
}

/// Record size written in the header (one record: a push message is at most 4 KB).
const RECORD_SIZE: u32 = 4096;
/// The longest plaintext (86-byte header, the text, the 0x02 delimiter, the 16-byte tag in 4096 bytes).
pub const MAX_PLAINTEXT: usize = 4096 - 86 - 1 - 16;

/// The content key and nonce of the only record (RFC 8291 section 3.4, RFC 8188 section 2.2).
fn keys(shared: &[u8], ua_public: &[u8], as_public: &[u8], auth: &[u8], salt: &[u8]) -> ([u8; 16], [u8; 12]) {
    let info = [b"WebPush: info\0".as_slice(), ua_public, as_public].concat();
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(auth), shared).expand(&info, &mut ikm).expect("32 bytes");
    let h = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let (mut cek, mut nonce) = ([0u8; 16], [0u8; 12]);
    h.expand(b"Content-Encoding: aes128gcm\0", &mut cek).expect("16 bytes");
    h.expand(b"Content-Encoding: nonce\0", &mut nonce).expect("12 bytes");
    (cek, nonce)
}

/// The request body for one message to a subscription (`ua_public`: its p256dh, 65 bytes; `auth`: 16 bytes).
/// `sender` and `salt` are fresh per message (`encrypt_fresh`); tests fix them.
pub fn encrypt(plain: &[u8], ua_public: &[u8], auth: &[u8], sender: &p256::SecretKey, salt: &[u8; 16]) -> Result<Vec<u8>, &'static str> {
    if auth.len() != 16 {
        return Err("bad key");
    }
    if plain.len() > MAX_PLAINTEXT {
        return Err("too long");
    }
    let ua = p256::PublicKey::from_sec1_bytes(ua_public).map_err(|_| "bad key")?;
    let ua_bytes = ua.to_encoded_point(false);
    let as_public = sender.public_key().to_encoded_point(false);
    let shared = p256::ecdh::diffie_hellman(sender.to_nonzero_scalar(), ua.as_affine());
    let (cek, nonce) = keys(shared.raw_secret_bytes(), ua_bytes.as_bytes(), as_public.as_bytes(), auth, salt);
    let cipher = aes_gcm::Aes128Gcm::new_from_slice(&cek).map_err(|_| "bad key")?;
    let msg = [plain, &[0x02]].concat();
    let sealed = cipher.encrypt(aes_gcm::Nonce::from_slice(&nonce), Payload { msg: &msg, aad: b"" }).map_err(|_| "encrypt")?;
    let mut out = salt.to_vec();
    out.extend(RECORD_SIZE.to_be_bytes());
    out.push(as_public.as_bytes().len() as u8);
    out.extend(as_public.as_bytes());
    out.extend(sealed);
    Ok(out)
}

pub fn encrypt_fresh(plain: &[u8], ua_public: &[u8], auth: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    encrypt(plain, ua_public, auth, &p256::SecretKey::random(&mut OsRng), &salt)
}

/// The receiver's side, for tests.
pub fn decrypt(body: &[u8], ua: &p256::SecretKey, auth: &[u8]) -> Option<Vec<u8>> {
    let salt = body.get(0..16)?;
    let n = *body.get(20)? as usize;
    let as_public = body.get(21..21 + n)?;
    let sender = p256::PublicKey::from_sec1_bytes(as_public).ok()?;
    let shared = p256::ecdh::diffie_hellman(ua.to_nonzero_scalar(), sender.as_affine());
    let (cek, nonce) = keys(shared.raw_secret_bytes(), ua.public_key().to_encoded_point(false).as_bytes(), as_public, auth, salt);
    let cipher = aes_gcm::Aes128Gcm::new_from_slice(&cek).ok()?;
    let mut plain = cipher.decrypt(aes_gcm::Nonce::from_slice(&nonce), body.get(21 + n..)?).ok()?;
    while plain.last() == Some(&0) {
        plain.pop();
    }
    (plain.pop() == Some(0x02)).then_some(plain)
}

/// The VAPID contact every push service sees: the project page, never the helper's own address.
pub const SUBJECT: &str = "https://github.com/minchenlee/flockbook";

/// `Authorization: vapid t=<jwt>, k=<public key>` (RFC 8292). `audience`: the endpoint's origin. Valid 12 hours.
pub fn vapid(audience: &str, subject: &str, key: &p256::ecdsa::SigningKey, now_s: u64) -> String {
    let head = b64url(br#"{"alg":"ES256","typ":"JWT"}"#);
    let claims = json!({"aud": audience, "exp": now_s + 12 * 3600, "sub": subject}).to_string();
    let input = format!("{head}.{}", b64url(claims.as_bytes()));
    let sig: p256::ecdsa::Signature = key.sign(input.as_bytes());
    let public = key.verifying_key().to_encoded_point(false);
    format!("vapid t={input}.{}, k={}", b64url(&sig.to_bytes()), b64url(public.as_bytes()))
}

// MARK: subscriptions

/// Chrome / Android (FCM), Safari / iOS (Apple), Firefox (Mozilla) and Edge (Windows).
pub const PUSH_HOSTS: [&str; 5] = ["fcm.googleapis.com", "android.googleapis.com", "push.apple.com", "push.services.mozilla.com", "notify.windows.com"];

/// The endpoint's origin ("https://host") when it is an https URL of a known push service, with no user and no
/// other port; else None. Nothing else is ever contacted.
pub fn allowed(endpoint: &str) -> Option<String> {
    // ASCII only: a non-ASCII host could pass the suffix check and become another name in curl (IDNA).
    if endpoint.len() > 2048 || !endpoint.is_ascii() || endpoint.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let rest = endpoint.strip_prefix("https://")?;
    let auth = rest.split(['/', '?', '#']).next()?;
    if auth.contains('@') || auth.contains('\\') {
        return None;
    }
    let host = match auth.rsplit_once(':') {
        Some((h, "443")) => h,
        Some(_) => return None,
        None => auth,
    }
    .to_ascii_lowercase();
    PUSH_HOSTS.iter().any(|p| host == *p || host.ends_with(&format!(".{p}"))).then(|| format!("https://{host}"))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sub {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub added: u64,
}

impl Sub {
    pub fn valid(&self) -> bool {
        allowed(&self.endpoint).is_some() && unb64url(&self.p256dh).is_some_and(|k| k.len() == 65) && unb64url(&self.auth).is_some_and(|a| a.len() == 16)
    }
    fn json(&self) -> Value {
        json!({"endpoint": self.endpoint, "p256dh": self.p256dh, "auth": self.auth, "added": self.added})
    }
}

/// Phones kept (one owner); the oldest goes past it.
pub const MAX_SUBS: usize = 10;

/// A file only the owner can read: a temp file made mode 600, then renamed over `path`. The folder is made 700.
pub fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no folder"))?;
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut rnd = [0u8; 4];
    OsRng.fill_bytes(&mut rnd);
    let tmp = dir.join(format!(".{}.{}.{}.tmp", path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(), std::process::id(), u32::from_le_bytes(rnd)));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let mut f = o.open(&tmp)?;
    let done = f.write_all(data).and_then(|_| f.sync_all());
    drop(f);
    done.and_then(|_| std::fs::rename(&tmp, path)).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Where push keeps its files.
pub fn folder(home: &Path) -> PathBuf {
    home.join("flockbook").join("serve")
}

/// The key in `vapid.key`, or a new one written there. Err when the file is there but holds no key (a new key would
/// silently break every phone's subscription), or cannot be written.
pub fn load_key(dir: &Path) -> Result<p256::ecdsa::SigningKey, String> {
    let path = dir.join("vapid.key");
    match std::fs::read_to_string(&path) {
        Ok(s) => unb64url(&s).and_then(|raw| p256::ecdsa::SigningKey::from_slice(&raw).ok()).ok_or_else(|| format!("{} holds no key", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = p256::ecdsa::SigningKey::random(&mut OsRng);
            write_private(&path, b64url(&key.to_bytes()).as_bytes()).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            Ok(key)
        }
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// The phones' subscriptions, in `subscriptions.json`.
pub struct Store {
    path: PathBuf,
    pub all: Vec<Sub>,
}

impl Store {
    /// None yet gives none. Err when the file is there but cannot be read as subscriptions: it is never overwritten
    /// (push stays off until the owner looks at it).
    pub fn open(dir: &Path) -> Result<Store, String> {
        let path = dir.join("subscriptions.json");
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Store { path, all: vec![] }),
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        };
        let bad = || format!("{} holds no subscriptions", path.display());
        let list = serde_json::from_slice::<Value>(&data).ok().and_then(|v| v.as_array().cloned()).ok_or_else(bad)?;
        let all = list
            .iter()
            .filter_map(|s| {
                Some(Sub { endpoint: s["endpoint"].as_str()?.into(), p256dh: s["p256dh"].as_str()?.into(), auth: s["auth"].as_str()?.into(), added: s["added"].as_u64().unwrap_or(0) })
            })
            .filter(Sub::valid)
            .collect();
        Ok(Store { path, all })
    }

    fn save(&self) -> bool {
        let v = Value::Array(self.all.iter().map(Sub::json).collect());
        write_private(&self.path, serde_json::to_string_pretty(&v).unwrap_or_default().as_bytes()).is_ok()
    }

    /// Adds or replaces (same endpoint) `s`. False: not valid, or not saved; nothing kept.
    pub fn add(&mut self, s: Sub) -> bool {
        if !s.valid() {
            return false;
        }
        let before = self.all.clone();
        self.all.retain(|x| x.endpoint != s.endpoint);
        self.all.push(s);
        while self.all.len() > MAX_SUBS {
            self.all.remove(0);
        }
        if self.save() {
            return true;
        }
        self.all = before;
        false
    }

    /// Gone from memory at once; the file follows (a failed save is tried again at the next change).
    pub fn remove(&mut self, endpoint: &str) {
        self.all.retain(|x| x.endpoint != endpoint);
        self.save();
    }
}

// MARK: when to push

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    NeedsYou,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Send it, with `more` held messages to mention.
    Send { more: usize },
    /// The same agent and kind went within `PER_AGENT`.
    Repeated,
    /// Over `PER_MINUTE`: held, `flush` tells of it later.
    Held,
}

/// One kind per agent at most once in `PER_AGENT`; at most `PER_MINUTE` pushes in any 60 s (`PushThrottle`).
/// Needs-you is never held: only the per-agent limit applies to it.
#[derive(Default)]
pub struct Throttle {
    last: HashMap<(Kind, String), Instant>,
    sent: Vec<Instant>,
    held: usize,
}

pub const PER_AGENT: Duration = Duration::from_secs(120);
pub const PER_MINUTE: usize = 6;

impl Throttle {
    fn room(&mut self, now: Instant) -> bool {
        self.sent.retain(|t| now.duration_since(*t) < Duration::from_secs(60));
        self.sent.len() < PER_MINUTE
    }

    pub fn decide(&mut self, agent: &str, kind: Kind, now: Instant) -> Decision {
        let key = (kind, agent.to_string());
        if self.last.get(&key).is_some_and(|t| now.duration_since(*t) < PER_AGENT) {
            return Decision::Repeated;
        }
        if kind == Kind::NeedsYou {
            self.room(now);
            self.last.insert(key, now);
            self.sent.push(now);
            return Decision::Send { more: 0 };
        }
        if !self.room(now) {
            self.held += 1;
            return Decision::Held;
        }
        self.last.insert(key, now);
        self.sent.push(now);
        Decision::Send { more: std::mem::take(&mut self.held) }
    }

    /// Held messages to tell of once the minute has room again (0: none).
    pub fn flush(&mut self, now: Instant) -> usize {
        if self.held == 0 || !self.room(now) {
            return 0;
        }
        self.sent.push(now);
        std::mem::take(&mut self.held)
    }
}

/// At most `chars` characters and `bytes` UTF-8 bytes, cut between characters; control characters dropped (a
/// newline kept when `newlines`).
fn clip(s: &str, chars: usize, bytes: usize, newlines: bool) -> String {
    let (mut out, mut n) = (String::new(), 0);
    for c in s.chars() {
        if (c.is_control() && !(newlines && c == '\n')) || c == '\u{7f}' {
            continue;
        }
        if n >= chars || out.len() + c.len_utf8() > bytes {
            break;
        }
        out.push(c);
        n += 1;
    }
    out
}

/// One push message for the page's service worker (`web/src/sw.ts`, `PushNotice`).
pub fn notice(title: &str, body: &str, tag: Option<&str>, pane: Option<&str>) -> Value {
    let mut v = json!({"title": clip(title, 120, 300, false), "body": clip(body, 200, 600, true)});
    if let Some(t) = tag {
        v["tag"] = json!(clip(t, 200, 200, false));
    }
    if let Some(p) = pane {
        v["pane"] = json!(clip(p, 200, 200, false));
    }
    v
}

/// Status changes worth a push (`PushTrigger`): a pane turning blocked (needs you), or done after working or blocked.
/// The first snapshot only records.
#[derive(Default)]
pub struct Trigger {
    last: Option<HashMap<String, AgentStatus>>,
}

pub struct Event {
    pub kind: Kind,
    pub pane: String,
    pub notice: Value,
}

impl Trigger {
    pub fn update(&mut self, workspaces: &[Workspace]) -> Vec<Event> {
        let mut now = HashMap::new();
        let mut out = vec![];
        for w in workspaces {
            for t in &w.tabs {
                for p in &t.panes {
                    let Some(a) = &p.agent else { continue };
                    now.insert(p.id.clone(), a.status);
                    let Some(last) = &self.last else { continue };
                    let was = last.get(&p.id).copied();
                    let kind = match a.status {
                        AgentStatus::Blocked if was != Some(AgentStatus::Blocked) => Kind::NeedsYou,
                        AgentStatus::Done if matches!(was, Some(AgentStatus::Working | AgentStatus::Blocked)) => Kind::Done,
                        _ => continue,
                    };
                    let what = if kind == Kind::NeedsYou { "Needs you" } else { "Done" };
                    let name = if a.title.is_empty() { &t.label } else { &a.title };
                    let body = [a.name.as_str(), &w.label, &t.label].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · ");
                    out.push(Event { kind, pane: p.id.clone(), notice: notice(&format!("{what} · {name}"), &body, Some(&p.id), Some(&p.id)) });
                }
            }
        }
        self.last = Some(now);
        out
    }
}

// MARK: sending

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Sent,
    /// 404 or 410: delete the subscription.
    Gone,
    /// Any other status (0: no answer).
    Failed(u16),
}

/// The headers of one push request (besides the body): what `curl` sends.
pub fn headers(s: &Sub, key: &p256::ecdsa::SigningKey, high: bool, now_s: u64) -> Option<Vec<String>> {
    let origin = allowed(&s.endpoint)?;
    Some(vec![
        "Content-Encoding: aes128gcm".into(),
        "Content-Type: application/octet-stream".into(),
        "TTL: 3600".into(),
        format!("Urgency: {}", if high { "high" } else { "normal" }),
        format!("Authorization: {}", vapid(&origin, SUBJECT, key, now_s)),
    ])
}

/// curl's options: no ~/.curlrc (`-q`, first), https only, no redirect, 30 s, the status code out; the rest of the
/// request (`curl_config`) comes on stdin (`-K -`). The endpoint and the VAPID token are not on the command line, which
/// every user of the machine can read (`ps`).
pub const CURL_ARGS: [&str; 16] = ["-q", "-sS", "--proto", "=https", "--max-redirs", "0", "-m", "30", "-o", "/dev/null", "-w", "%{http_code}", "-X", "POST", "-K", "-"];

/// The config curl reads on stdin: the URL, the headers, and the body from `body` (a file only the owner can read).
pub fn curl_config(endpoint: &str, headers: &[String], body: &Path) -> String {
    let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace(['\n', '\r'], ""));
    let mut c = format!("url = {}\n", q(endpoint));
    for h in headers {
        c += &format!("header = {}\n", q(h));
    }
    c + &format!("data-binary = {}\n", q(&format!("@{}", body.display())))
}

/// Posts one encrypted message with curl (`CURL_ARGS`). The body waits in `dir` (mode 600) while curl runs. The HTTP
/// status; 0: no answer.
pub fn curl_post(dir: &Path, endpoint: &str, headers: &[String], body: &[u8]) -> u16 {
    use std::io::Write;
    use std::process::{Command, Stdio};
    if allowed(endpoint).is_none() {
        return 0;
    }
    let mut rnd = [0u8; 8];
    OsRng.fill_bytes(&mut rnd);
    let file = dir.join(format!(".push-{}-{}.body", std::process::id(), b64url(&rnd)));
    if write_private(&file, body).is_err() {
        return 0;
    }
    let status = (|| {
        let mut child = Command::new("curl").args(CURL_ARGS).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
        let mut i = child.stdin.take()?;
        let _ = i.write_all(curl_config(endpoint, headers, &file).as_bytes());
        drop(i);
        String::from_utf8_lossy(&child.wait_with_output().ok()?.stdout).trim().parse().ok()
    })();
    let _ = std::fs::remove_file(&file);
    status.unwrap_or(0)
}

pub fn outcome(status: u16) -> Outcome {
    match status {
        200..=299 => Outcome::Sent,
        404 | 410 => Outcome::Gone,
        s => Outcome::Failed(s),
    }
}

/// Posts `body` with `headers` to `endpoint`; the HTTP status. The real one is `curl_post`.
pub type Transport = dyn Fn(&str, &[String], &[u8]) -> u16 + Send + Sync;

/// Push for one helper: its key, the subscriptions, the throttle and the trigger.
pub struct Center {
    pub key: p256::ecdsa::SigningKey,
    pub store: Store,
    throttle: Throttle,
    trigger: Trigger,
}

impl Center {
    pub fn new(key: p256::ecdsa::SigningKey, store: Store) -> Center {
        Center { key, store, throttle: Throttle::default(), trigger: Trigger::default() }
    }

    /// The key the page subscribes with (base64url, 65 bytes).
    pub fn public_key(&self) -> String {
        b64url(self.key.verifying_key().to_encoded_point(false).as_bytes())
    }

    /// The messages to send after this snapshot (and whether each is urgent): its changes, throttled, then held ones
    /// once there is room.
    pub fn plan(&mut self, workspaces: &[Workspace], now: Instant) -> Vec<(Value, bool)> {
        let mut out = vec![];
        for e in self.trigger.update(workspaces) {
            if let Decision::Send { more } = self.throttle.decide(&e.pane, e.kind, now) {
                let mut n = e.notice;
                if more > 0 {
                    let b = n["body"].as_str().unwrap_or("").to_string();
                    n = notice(n["title"].as_str().unwrap_or(""), &if b.is_empty() { format!("+{more} more") } else { format!("{b} · +{more} more") }, n["tag"].as_str(), n["pane"].as_str());
                }
                out.push((n, e.kind == Kind::NeedsYou));
            }
        }
        let held = self.throttle.flush(now);
        if held > 0 {
            out.push((notice("Flockbook", &format!("{held} more agent updates. Open Flockbook to see them."), Some("more"), None), false));
        }
        out
    }
}

/// Posts each message to each phone. The endpoints that are gone (404, 410).
pub fn deliver(subs: &[Sub], key: &p256::ecdsa::SigningKey, messages: &[(Value, bool)], transport: &Transport) -> Vec<String> {
    let now_s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut gone: Vec<String> = vec![];
    for (n, high) in messages {
        let payload = n.to_string().into_bytes();
        for s in subs {
            if gone.contains(&s.endpoint) {
                continue;
            }
            let (Some(ua), Some(auth), Some(h)) = (unb64url(&s.p256dh), unb64url(&s.auth), headers(s, key, *high, now_s)) else { continue };
            let Ok(body) = encrypt_fresh(&payload, &ua, &auth) else { continue };
            if outcome(transport(&s.endpoint, &h, &body)) == Outcome::Gone {
                gone.push(s.endpoint.clone());
            }
        }
    }
    gone
}

fn locked(c: &std::sync::Mutex<Option<Center>>) -> std::sync::MutexGuard<'_, Option<Center>> {
    c.lock().unwrap_or_else(|e| e.into_inner())
}

/// One round of the loop. The lock is held only to plan and to remove gone phones, never while curl runs (up to 30 s
/// a phone): a page's hello and (un)subscribe take it too.
pub fn round(center: &std::sync::Mutex<Option<Center>>, workspaces: &[Workspace], now: Instant, transport: &Transport) {
    let (messages, subs, key) = {
        let mut g = locked(center);
        let Some(c) = g.as_mut() else { return };
        let m = c.plan(workspaces, now);
        if m.is_empty() || c.store.all.is_empty() {
            return;
        }
        (m, c.store.all.clone(), c.key.clone())
    };
    let gone = deliver(&subs, &key, &messages, transport);
    if let (false, Some(c)) = (gone.is_empty(), locked(center).as_mut()) {
        for e in gone {
            c.store.remove(&e);
        }
    }
}

/// The helper's push center; None when push is off (no home, or a key file that holds no key).
pub static CENTER: std::sync::Mutex<Option<Center>> = std::sync::Mutex::new(None);

pub fn center() -> std::sync::MutexGuard<'static, Option<Center>> {
    locked(&CENTER)
}

/// Starts push for `home`: the key, the subscriptions, and the loop that reads the workspaces every `every`.
/// Err (push stays off): why.
pub fn start(home: &Path, every: Duration, transport: Box<Transport>, read: impl Fn() -> Vec<Workspace> + Send + 'static) -> Result<(), String> {
    let dir = folder(home);
    let key = load_key(&dir)?;
    let store = Store::open(&dir)?;
    *center() = Some(Center::new(key, store));
    std::thread::spawn(move || loop {
        let ws = read();
        round(&CENTER, &ws, Instant::now(), transport.as_ref());
        std::thread::sleep(every);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use herdr_core::{Agent, Pane, Tab};

    fn b(s: &str) -> Vec<u8> {
        unb64url(s).unwrap()
    }

    #[test]
    fn rfc8291_example() {
        let plain = b"When I grow up, I want to be a watermelon";
        let sender = p256::SecretKey::from_slice(&b("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")).unwrap();
        let ua_public = b("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4");
        let ua = p256::SecretKey::from_slice(&b("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94")).unwrap();
        let salt: [u8; 16] = b("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let auth = b("BTBZMqHH6r4Tts7J_aSIgg");
        let expected = b(concat!(
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27ml",
            "mlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPT",
            "pK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
        ));
        assert_eq!(expected.len(), 144);
        assert_eq!(encrypt(plain, &ua_public, &auth, &sender, &salt).unwrap(), expected, "byte for byte");
        let fresh = encrypt_fresh(plain, &ua_public, &auth).unwrap();
        assert_ne!(&fresh[..16], &salt[..], "a fresh salt");
        assert_eq!(decrypt(&fresh, &ua, &auth).unwrap(), plain, "the phone reads a fresh one");
        assert!(encrypt(&vec![0; MAX_PLAINTEXT + 1], &ua_public, &auth, &sender, &salt).is_err());
        assert!(encrypt(plain, &ua_public, &auth[..15], &sender, &salt).is_err());
    }

    #[test]
    fn vapid_is_signed() {
        use p256::ecdsa::signature::Verifier;
        let key = p256::ecdsa::SigningKey::random(&mut OsRng);
        let a = vapid("https://fcm.googleapis.com", SUBJECT, &key, 1_793_670_000);
        let (t, k) = a.strip_prefix("vapid t=").unwrap().split_once(", k=").unwrap();
        assert_eq!(b(k), key.verifying_key().to_encoded_point(false).as_bytes());
        let (input, sig) = t.rsplit_once('.').unwrap();
        let sig = p256::ecdsa::Signature::from_slice(&b(sig)).unwrap();
        key.verifying_key().verify(input.as_bytes(), &sig).unwrap();
        let claims: Value = serde_json::from_slice(&b(input.split('.').nth(1).unwrap())).unwrap();
        assert_eq!(claims, json!({"aud": "https://fcm.googleapis.com", "exp": 1_793_670_000u64 + 43_200, "sub": SUBJECT}));
    }

    #[test]
    fn only_push_services() {
        assert_eq!(allowed("https://fcm.googleapis.com/fcm/send/abc").as_deref(), Some("https://fcm.googleapis.com"));
        assert_eq!(allowed("https://web.push.apple.com:443/QG").as_deref(), Some("https://web.push.apple.com"));
        assert!(allowed("https://updates.push.services.mozilla.com/wpush/v2/x").is_some());
        for bad in [
            "http://fcm.googleapis.com/x",
            "https://fcm.googleapis.com.evil.example/x",
            "https://evilfcm.googleapis.com/x",
            "https://user@fcm.googleapis.com/x",
            "https://fcm.googleapis.com:8443/x",
            "https://127.0.0.1/x",
            "https://fcm.googleapis.com/x y",
            "https://example.com/?fcm.googleapis.com",
            "https://ex\u{0430}mple.fcm.googleapis.com/x",
            "https://fcm.googleapis.com\u{3002}evil.example/x",
        ] {
            assert!(allowed(bad).is_none(), "{bad}");
        }
        assert!(allowed(&format!("https://fcm.googleapis.com/{}", "a".repeat(2048))).is_none());
    }

    fn sub(endpoint: &str) -> Sub {
        let ua = p256::SecretKey::random(&mut OsRng);
        Sub { endpoint: endpoint.into(), p256dh: b64url(ua.public_key().to_encoded_point(false).as_bytes()), auth: b64url(&[7; 16]), added: 1 }
    }

    #[cfg(unix)]
    #[test]
    fn files_are_private_and_a_bad_key_is_kept() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("serve");
        let k1 = load_key(&d).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(&d), mode(&d.join("vapid.key"))), (0o700, 0o600));
        assert_eq!(load_key(&d).unwrap().to_bytes(), k1.to_bytes(), "the same key next time");
        let mut s = Store::open(&d).unwrap();
        assert!(s.add(sub("https://fcm.googleapis.com/a")));
        assert!(!s.add(sub("https://example.com/a")), "not a push service");
        assert_eq!(mode(&d.join("subscriptions.json")), 0o600);
        assert_eq!(Store::open(&d).unwrap().all, s.all);
        for i in 0..MAX_SUBS + 2 {
            s.add(sub(&format!("https://fcm.googleapis.com/{i}")));
        }
        assert_eq!(s.all.len(), MAX_SUBS);
        std::fs::write(d.join("subscriptions.json"), "{oops").unwrap();
        assert!(Store::open(&d).is_err(), "a broken file is refused");
        assert_eq!(std::fs::read_to_string(d.join("subscriptions.json")).unwrap(), "{oops");
        std::fs::write(d.join("vapid.key"), "not a key").unwrap();
        assert!(load_key(&d).is_err());
        assert_eq!(std::fs::read_to_string(d.join("vapid.key")).unwrap(), "not a key", "never overwritten");
    }

    #[test]
    fn throttle() {
        let t0 = Instant::now();
        let mut t = Throttle::default();
        assert_eq!(t.decide("p1", Kind::Done, t0), Decision::Send { more: 0 });
        assert_eq!(t.decide("p1", Kind::Done, t0 + Duration::from_secs(30)), Decision::Repeated);
        assert_eq!(t.decide("p1", Kind::NeedsYou, t0), Decision::Send { more: 0 });
        for i in 2..6 {
            assert_eq!(t.decide(&format!("p{i}"), Kind::Done, t0), Decision::Send { more: 0 });
        }
        assert_eq!(t.decide("p6", Kind::Done, t0), Decision::Held);
        assert_eq!(t.decide("p7", Kind::NeedsYou, t0), Decision::Send { more: 0 }, "needs you is never held");
        assert_eq!(t.flush(t0), 0, "no room yet");
        assert_eq!(t.flush(t0 + Duration::from_secs(61)), 1);
    }

    fn ws(status: AgentStatus) -> Vec<Workspace> {
        let agent = Agent { name: "claude".into(), kind: "claude".into(), status, title: "Fix the build".into(), ..Default::default() };
        let pane = Pane { id: "hc:t1".into(), terminal_id: "t1".into(), agent: Some(agent), ..Default::default() };
        vec![Workspace { id: "w".into(), label: "site".into(), tabs: vec![Tab { id: "t".into(), label: "build".into(), panes: vec![pane], ..Default::default() }] }]
    }

    #[test]
    fn curl_gets_the_secrets_on_stdin() {
        assert_eq!(CURL_ARGS[0], "-q", "no ~/.curlrc");
        assert!(CURL_ARGS.windows(2).any(|w| w == ["--max-redirs", "0"]) && CURL_ARGS.windows(2).any(|w| w == ["--proto", "=https"]));
        assert_eq!(&CURL_ARGS[14..], ["-K", "-"]);
        let h = vec!["Authorization: vapid t=SECRET.JWT, k=KEY".to_string(), "TTL: 3600".into()];
        let c = curl_config("https://fcm.googleapis.com/x\"y\\z", &h, Path::new("/home/you/b"));
        assert_eq!(c, "url = \"https://fcm.googleapis.com/x\\\"y\\\\z\"\nheader = \"Authorization: vapid t=SECRET.JWT, k=KEY\"\nheader = \"TTL: 3600\"\ndata-binary = \"@/home/you/b\"\n");
        assert!(!curl_config("https://fcm.googleapis.com/x", &["a\nurl = https://evil".into()], Path::new("/b")).contains("\nurl = https://evil"), "one line each");
    }

    #[test]
    fn pushes_on_blocked_and_done_without_conversation() {
        type Posted = Vec<(String, Vec<String>, Vec<u8>)>;
        let sent: std::sync::Arc<std::sync::Mutex<Posted>> = Default::default();
        let status = std::sync::Arc::new(std::sync::Mutex::new(201u16));
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let ua = p256::SecretKey::random(&mut OsRng);
        let phone = Sub { endpoint: "https://fcm.googleapis.com/fcm/send/x".into(), p256dh: b64url(ua.public_key().to_encoded_point(false).as_bytes()), auth: b64url(&[7; 16]), added: 1 };
        assert!(store.add(phone.clone()));
        let center = std::sync::Arc::new(std::sync::Mutex::new(Some(Center::new(p256::ecdsa::SigningKey::random(&mut OsRng), store))));
        let (s2, st2, c2) = (sent.clone(), status.clone(), center.clone());
        let transport = move |e: &str, h: &[String], b: &[u8]| {
            assert!(c2.try_lock().is_ok(), "the lock is free while a push goes out");
            s2.lock().unwrap().push((e.to_string(), h.to_vec(), b.to_vec()));
            *st2.lock().unwrap()
        };
        let t0 = Instant::now();
        round(&center, &ws(AgentStatus::Blocked), t0, &transport);
        assert!(sent.lock().unwrap().is_empty(), "the first snapshot only records");
        round(&center, &ws(AgentStatus::Working), t0, &transport);
        round(&center, &ws(AgentStatus::Blocked), t0, &transport);
        {
            let got = sent.lock().unwrap();
            assert_eq!(got.len(), 1);
            let (e, h, body) = &got[0];
            assert_eq!(e, &phone.endpoint);
            assert!(h.contains(&"Urgency: high".to_string()) && h.iter().any(|x| x.starts_with("Authorization: vapid t=")));
            let n: Value = serde_json::from_slice(&decrypt(body, &ua, &[7; 16]).unwrap()).unwrap();
            assert_eq!(n, json!({"title": "Needs you · Fix the build", "body": "claude · site · build", "tag": "hc:t1", "pane": "hc:t1"}));
        }
        // The phone unsubscribed: 410 removes it, nothing more goes there.
        *status.lock().unwrap() = 410;
        round(&center, &ws(AgentStatus::Done), t0, &transport);
        assert_eq!(sent.lock().unwrap().len(), 2, "done after blocked");
        assert!(center.lock().unwrap().as_ref().unwrap().store.all.is_empty());
        assert!(Store::open(dir.path()).unwrap().all.is_empty(), "gone from the file too");
        round(&center, &ws(AgentStatus::Working), t0, &transport);
        round(&center, &ws(AgentStatus::Blocked), t0 + PER_AGENT, &transport);
        assert_eq!(sent.lock().unwrap().len(), 2, "no phone left");
    }

}
