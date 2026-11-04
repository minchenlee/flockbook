// Flockbook PWA: pair, agent list (Needs you first), conversation, approval and question cards, composer (input only).
// Talks to the hub on the same origin (`/ws`). Plain DOM, no packages. Remote protocol v1.
                                                                                                                                                                     
import { inline, markdown } from "./markdown.js";
import { monogram } from "./monogram.js";

const TOKEN_KEY = "flockbook.token", SIDE_KEY = "flockbook.side";
const app = document.getElementById("app")               ;
/** An iPad's width (portrait too) and up: two columns (see `split`). A phone on its side is as wide but low: one column. */
const wideScreen = matchMedia("(min-width: 768px) and (min-height: 600px)");
// The row being read stays where it is (the resize before this put it back, see `seenRow`).
wideScreen.addEventListener("change", () => { keepTopRow = !following; render(); });

// --- Storage (may be blocked: private mode, cleared site data).
function load(key        )                {
  try { return localStorage.getItem(key); } catch { return null; }
}
function save(key        , value               )       {
  try { value === null ? localStorage.removeItem(key) : localStorage.setItem(key, value); } catch { /* not kept */ }
}

// --- DOM helper.
                                                      
function h                                       (tag   , props                          = {}, ...children         )                           {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v === undefined || v === null || v === false) continue;
    if (k === "class") el.className = String(v);
    else if (k.startsWith("on") && typeof v === "function") el.addEventListener(k.slice(2), v                 );
    else if (k in el && typeof v !== "string") (el                                      )[k] = v;
    else el.setAttribute(k, String(v));
  }
  for (const c of children) if (c !== null && c !== undefined && c !== false) el.append(c);
  return el;
}

// --- State.
/** An image picked for the next message. `uploaded`: the hub holds it under `name` on this connection. */
                                                                                     
const MAX_IMAGES = 4, MAX_IMAGE_BYTES = 10 * 1024 * 1024, PART_BYTES = 640 * 1024;
/** `waiting`: older rows that came in and go in front of `items` a few per frame (oldest first). */
                                                                                                                                                                                                       
const state = {
  token: load(TOKEN_KEY),
  pairSecret: null                 ,
  fingerprint: null                 ,
  pairError: null                 ,
  connected: false,
  welcomed: false,
  hub: "",
  level: "read"         ,
  workspaces: []               ,
  panes: new Map                  (),
  watching: null                 ,
  busy: false,
  banner: null                                         ,
  toast: null                 ,
  /** Images picked per pane, not sent yet. */
  attachments: new Map                      (),
  /** Upload progress of the message being sent, 0…1; null when none. */
  progress: null                 ,
  /** The hub's Web Push key (from `welcome`); null: the hub sends no notifications. */
  pushKey: null                 ,
  /** This device gets notifications; `busy` while turning them on or off. */
  notify: "off"                         ,
  /** Texts the hub pulled back from agents' queues, per pane, oldest first, for that pane's composer. */
  fills: {}                                                                                         ,
  /** Unsent composer text per pane, and the pane whose composer is on the page: switching agents never carries one
   *  agent's text into another's composer. */
  drafts: {}                          ,
  composerPane: null                 ,
  /** Bumped on every change of a pane's composer text (typed, or a pulled text placed): a send clears the draft only
   *  when it is still the one it sent. */
  revs: {}                          ,
  /** Ids of pulled texts already in a composer: one the hub sends again (no ack reached it) is not added twice. */
  placed: new Set        (),
  /** Panes whose Claude Code status line is open: every tag and when it was updated. */
  statusOpen: new Set        (),
  /** Each pane's Screen as the reader left it, open or closed; a pane not in it shows its default. A hub tick re-renders
   *  the page and builds the Screen again: without this it took its default back. */
  screenOpen: new Map                 (),
  /** Tool chips the reader opened (`<pane> <row id> <chip index>`): a running call changes its row, which is built again. */
  chipsOpen: new Set        (),
  /** On a wide screen, the reader hid the agents beside the conversation (kept on this device). */
  sideHidden: load(SIDE_KEY) === "hidden",
};

// The QR link: `#pair=<secret>&fp=<sha256>`. Read once, then removed from the address bar.
function readPairLink()          {
  const params = new URLSearchParams(location.hash.slice(1));
  if (!params.get("pair")) return false;
  state.pairSecret = params.get("pair");
  state.fingerprint = params.get("fp");
  state.pairError = null;
  history.replaceState(null, "", location.pathname);
  return true;
}
readPairLink();

// --- Socket.
let ws                   = null;
let nextId = 1;
let retry = 0;
const waiting = new Map                                                                ();

function connect()       {
  ws = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`);
  const sock = ws;
  sock.addEventListener("open", () => {
    state.connected = true;
    retry = 0;
    if (state.token) hello();
    render();
  });
  sock.addEventListener("message", (e                      ) => {
    let m            ;
    try { m = JSON.parse(e.data)              ; } catch { return; }
    receive(m);
  });
  sock.addEventListener("close", () => {
    if (ws !== sock) return;
    ws = null;
    state.connected = false;
    state.welcomed = false;
    cancelRound();
    // The hub drops a connection's uploads when it closes: send them again next time.
    for (const list of state.attachments.values()) for (const a of list) a.uploaded = false;
    for (const done of waiting.values()) done(false, "failed", "Not connected.");
    waiting.clear();
    render();
    // Reconnect while paired (1, 2, 4 … 15 s).
    if (state.token) setTimeout(connect, Math.min(15000, 1000 * 2 ** retry++));
  });
}

/** Resolves when the socket is open, or after 5 s. */
async function opened()                {
  for (let i = 0; i < 100 && ws?.readyState === WebSocket.CONNECTING; i++) await new Promise((r) => setTimeout(r, 50));
}

/** The composer has the pulled text `id`: the hub forgets it. Lost (no connection): the hub sends it again after
 *  hello, and `placed` answers with another ack. */
function ack(id        )       {
  void request({ type: "pulledAck", pulled: id });
}

/** The Rust helper (`flockbook-modfeed serve`, `/hub.json`): Tailscale says who the owner is, so there is no pairing,
 *  and the requests that send keys go with `POST /api` (same origin) instead of the socket. Null: the Mac hub. */
let helper                            = null;

/** A request over `POST /api`: the answer is the hub messages, the reply to `id` among them. */
async function post(msg                         , id        )                                                            {
  try {
    const r = await fetch("/api", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ v: 1, id, ...msg }),
      credentials: "same-origin", cache: "no-store" });
    if (!r.ok) return { ok: false, code: "failed", message: r.status === 403 ? "The helper refused this request." : `The helper answered ${r.status}.` };
    let reply                                                   = { ok: false, code: "failed", message: "The helper gave no answer." };
    for (const m of await r.json()                ) {
      if (m.type === "reply" && m.re === id) reply = { ok: m.ok, code: m.error?.code, message: m.error?.message };
      else receive(m);
    }
    return reply;
  } catch {
    return { ok: false, code: "failed", message: "Not connected." };
  }
}

function request(msg                         , id = `c${nextId++}`)                                                            {
  if (helper && helper.post.includes(String(msg.type))) return post(msg, id);
  return new Promise((resolve) => {
    if (!ws || ws.readyState !== WebSocket.OPEN) { resolve({ ok: false, code: "failed", message: "Not connected." }); return; }
    waiting.set(id, (ok, code, message) => resolve({ ok, code, message }));
    ws.send(JSON.stringify({ v: 1, id, ...msg }));
  });
}

async function hello()                {
  const r = await request({ type: "hello", token: state.token, client: "flockbook-web", clientVersion: "1" });
  if (!r.ok && r.code === "unauthorized") {
    state.token = null;
    save(TOKEN_KEY, null);
    state.pairError = "This device is not paired (anymore). Pair it again from the Mac.";
    render();
    return;
  }
  if (r.ok && state.watching) await request({ type: "watch", pane: state.watching });
  // Pictures in view that did not come while the connection was down.
  if (r.ok) for (const b of near) showPicture(b);
}

function view(pane        )           {
  let v = state.panes.get(pane);
  if (!v) { v = { items: [], hasEarlier: false }; state.panes.set(pane, v); }
  return v;
}

function receive(m            )       {
  switch (m.type) {
    case "reply": {
      const done = waiting.get(m.re);
      waiting.delete(m.re);
      done?.(m.ok, m.error?.code, m.error?.message);
      return;
    }
    case "paired":
      state.token = m.token;
      save(TOKEN_KEY, m.token);
      state.pairSecret = null;
      state.pairError = null;
      hello();
      return;
    case "welcome":
      state.welcomed = true;
      state.hub = m.hub;
      state.level = m.level;
      state.pushKey = m.pushKey ?? null;
      syncPush(m.pushOn === true);
      break;
    case "snapshot": {
      const before = new Set(blockedPanes(state.workspaces).map((p) => p.id));
      const first = state.workspaces.length === 0;
      state.workspaces = m.workspaces;
      if (!first) {
        const fresh = blockedPanes(m.workspaces).find((p) => !before.has(p.id) && p.id !== state.watching);
        if (fresh) state.banner = { pane: fresh.id, text: `${agentName(fresh)} needs you` };
      }
      break;
    }
    case "status":
      for (const w of state.workspaces) for (const t of w.tabs) for (const p of t.panes) if (p.id === m.pane && p.agent) p.agent.status = m.status;
      break;
    case "conversation": {
      const v = view(m.pane);
      // The pane shows another session (a new agent there): it opens at its newest message, like a switch.
      if (m.pane === state.watching && v.session !== undefined && v.session !== m.session) { cancelRound(); following = true; }
      if (v.session !== m.session) v.waiting = undefined;
      v.session = m.session;
      v.hasEarlier = m.hasEarlier;
      if (m.mode === "prepend" && m.pane === state.watching) keepTopRow = true;
      if (m.mode === "replace") { v.items = m.items; v.waiting = undefined; }
      else {
        const known = new Map(m.items.map((i) => [i.id, i]));
        const kept = v.items.map((i) => known.get(i.id) ?? i);
        const have = new Set([...v.items, ...(v.waiting ?? [])].map((i) => i.id));
        const added = m.items.filter((i) => !have.has(i.id));
        if (v.waiting) v.waiting = v.waiting.map((i) => known.get(i.id) ?? i);
        v.items = kept;
        // Older rows go in a few per frame (all at once, the page lays them all out in one long frame).
        if (m.mode === "prepend") { v.waiting = [...added, ...(v.waiting ?? [])]; addWaiting(m.pane); return; }
        v.items.push(...added);
      }
      break;
    }
    case "screen":
      view(m.pane).screen = m.text;
      break;
    case "statusLine":
      view(m.pane).statusLine = statusTags(m).length ? m : undefined;
      break;
    case "queued":
      view(m.pane).queue = m.items.length ? { agent: m.agent, items: m.items, pullable: m.pullable, pullsAll: m.pullsAll } : undefined;
      break;
    case "picturePart": {
      const p = pictureParts.get(m.re);
      if (!p || m.part < 0 || m.part >= m.parts || (p.parts.length && p.parts.length !== m.parts)) return;
      if (!p.parts.length) { p.parts = new Array        (m.parts); p.mime = m.mime; }
      if (p.parts[m.part] === undefined) p.got++;
      p.parts[m.part] = m.data;
      return;
    }
    case "pulled": {
      if (m.id && state.placed.has(m.id)) { ack(m.id); break; }
      const f = state.fills[m.pane] ??= [];
      if (m.id && f.some((x) => x.id === m.id)) break;
      // Its warning goes with it, shown when it goes into its own composer (maybe later, on that agent).
      f.push({ id: m.id, text: m.text, note: m.note, interrupted: m.interrupted });
      break;
    }
    case "prompt": {
      const v = view(m.pane);
      v.approval = m.approval;
      v.question = m.question;
      break;
    }
    default:
      return;
  }
  render();
}

// --- Notifications (Web Push). The hub's key comes with `welcome`; the hub binds the subscription to this device.
// iOS offers push only to a Home Screen app; Safari tabs have no PushManager.
const pushReady = ()          => "serviceWorker" in navigator && "PushManager" in window && "Notification" in window;
const iosTab = ()          => /iPhone|iPad|iPod/.test(navigator.userAgent) && !pushReady();
function worker()                                            {
  return "serviceWorker" in navigator ? navigator.serviceWorker.register("sw.js").catch(() => null) : Promise.resolve(null);
}
function keyBytes(b64url        )                          {
  const s = atob(b64url.replace(/-/g, "+").replace(/_/g, "/") + "=".repeat((4 - b64url.length % 4) % 4));
  return Uint8Array.from(s, (c) => c.charCodeAt(0));
}
function subscribeMessage(s                  )                          {
  const j = s.toJSON();
  return { type: "pushSubscribe", endpoint: j.endpoint, p256dh: j.keys?.p256dh, auth: j.keys?.auth };
}
/** After `welcome`: on when this browser's subscription is the one the hub keeps (`pushOn`, same key). Else drops it:
 *  the owner removed it on the Mac, or the hub has a new key. Never subscribes again by itself. */
async function syncPush(hubHasOne         )                {
  const reg = pushReady() ? await worker() : null;
  const s = await reg?.pushManager.getSubscription().catch(() => null);
  let on = false;
  if (s) {
    const had = new Uint8Array(s.options.applicationServerKey ?? new ArrayBuffer(0)), want = state.pushKey ? keyBytes(state.pushKey) : new Uint8Array(0);
    on = hubHasOne && had.length === want.length && had.every((b, i) => b === want[i]);
    if (!on && state.pushKey) await s.unsubscribe().catch(() => false);
  }
  state.notify = on ? "on" : "off";
  render();
}
async function togglePush()                {
  if (state.notify === "busy" || !state.pushKey) return;
  if (iosTab()) { toast("On iPhone, add Flockbook to the Home Screen first. Then turn on notifications there."); return; }
  if (!pushReady()) { toast("This browser cannot show notifications."); return; }
  const was = state.notify;
  state.notify = "busy";
  render();
  const reg = await worker();
  const old = await reg?.pushManager.getSubscription().catch(() => null);
  if (was === "on") {
    if (old) { await request({ type: "pushUnsubscribe", endpoint: old.endpoint }); await old.unsubscribe().catch(() => false); }
    state.notify = "off";
    toast("Notifications off.");
    return;
  }
  const allowed = await Notification.requestPermission().catch(() => "denied");
  if (allowed !== "granted" || !reg) {
    state.notify = "off";
    toast("Notifications are blocked. Allow them for this site in the browser settings.");
    return;
  }
  try {
    await navigator.serviceWorker.ready;
    const s = old ?? await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: keyBytes(state.pushKey) });
    const r = await request(subscribeMessage(s));
    if (!r.ok) { await s.unsubscribe().catch(() => false); throw new Error(explain(r.code, r.message)); }
    state.notify = "on";
    toast("Notifications on. You hear about agents that need you, even with this page closed.");
  } catch (e) {
    state.notify = "off";
    toast(`Notifications did not turn on. ${e instanceof Error ? e.message : ""}`.trim());
  }
}

// --- Navigation: `#pane=<id>` is the conversation; empty is the list.
function route()                {
  const p = new URLSearchParams(location.hash.slice(1)).get("pane");
  return p && p.length < 200 ? p : null;
}
async function follow()                {
  if (readPairLink()) { if (!state.token && !ws) connect(); render(); return; }
  const want = route();
  if (want === state.watching) { render(); return; }
  if (state.watching) request({ type: "unwatch", pane: state.watching });
  const from = state.watching;
  state.watching = want;
  cancelRound();
  following = true;   // an agent opens at its newest message
  if (state.banner && state.banner.pane === want) state.banner = null;
  navigate(from, want, render);
  if (want) await request({ type: "watch", pane: want });
}
addEventListener("hashchange", follow);

// --- The newest message. An open conversation stays at the bottom while rows arrive and lay out (markdown, images),
// until the reader scrolls up; scrolling back down follows again. Opening, switching or reloading starts at the bottom.
let following = true;
/** What the reader sees is the visual viewport: smaller with the keyboard open or zoomed in. */
function nearBottom()          {
  const v = window.visualViewport;
  const seen = v ? v.pageTop + v.height : window.scrollY + window.innerHeight;
  return document.documentElement.scrollHeight - seen <= 40;
}
function toBottom()       {
  if (state.watching && following && !reader.active) window.scrollTo(0, document.documentElement.scrollHeight);
}
/** Where the reader sees the page from (the visual viewport: it moves alone when zoomed in). */
const viewTop = ()         => window.visualViewport?.pageTop ?? window.scrollY;
/** Only the reader changes `following`. A reader action (wheel, touch, a scroll key, the scroll bar from press to
 * release) on a shown conversation starts a round; the round ends when the page has stopped scrolling for `settleMs`.
 * At its end: the reader reached the bottom during the round (scrolled down to it, pushed down at it, or pressed End)
 * and did not scroll up after (new rows may have come in meanwhile) -> following. Otherwise moved since the round
 * started -> not following; not moved -> as before. A page that got shorter and moved the view is not the reader. Every scroll outside a round (the page's own, a page that got shorter, a reconnect, the
 * background) leaves `following` alone. A round ends without a verdict when the conversation stops being shown
 * (disconnected, hidden, another page or agent). During a round nothing pulls the page down; at its end a following
 * page catches up. */
const reader = { active: false, holding: false, touching: false, from: 0, last: 0, reachedEnd: false, endTop: 0, settle: 0, still: 0, speed: 0, seenAt: 0 };
const settleMs = 150;
/** A release the page never hears (outside the window) ends the hold with the window's focus, or after 10 s without
 * a move. */
const stillMs = 10000;
const shown = ()          => !!state.watching && state.welcomed && document.visibilityState === "visible";
function cancelRound()       {
  clearTimeout(reader.settle);
  clearTimeout(reader.still);
  reader.active = reader.holding = reader.touching = reader.reachedEnd = false;
  // Older rows that waited for the round go in, after the change that ended it (another session drops them).
  setTimeout(() => { if (state.watching) addWaiting(state.watching); });
}
/** The bottom reached in this round: the view moved down to it. Kept until the view is more than 1 px above the
 *  lowest place it reached since (a page that got shorter moves it up too). */
function track()       {
  const top = viewTop(), now = performance.now(), dt = now - reader.seenAt;
  // How fast the reader scrolls up (px/s; 0 going down), for how far ahead older rows start to load.
  if (top >= reader.last || dt >= 250) reader.speed = 0;
  else if (dt > 2) reader.speed = ((reader.last - top) * 1000) / dt;
  reader.seenAt = now;
  if (top > reader.last && nearBottom()) reachEnd();
  else if (reader.reachedEnd) {
    if (top > reader.endTop) reader.endTop = top;
    else if (top < reader.endTop - 1) reader.reachedEnd = false;
  }
  reader.last = top;
}
function reachEnd()       {
  reader.endTop = reader.reachedEnd ? Math.max(reader.endTop, viewTop()) : viewTop();
  reader.reachedEnd = true;
}
function readerActs()       {
  if (!shown()) return;
  if (!reader.active) { reader.active = true; reader.from = reader.last = viewTop(); reader.reachedEnd = false; }
  track();
  clearTimeout(reader.settle);
  if (!reader.holding) reader.settle = setTimeout(settled, settleMs);
}
function settled()       {
  if (reader.holding) return;
  reader.active = false;
  if (reader.reachedEnd) following = true;
  else if (Math.abs(viewTop() - reader.from) >= 1) following = false;
  noteSeenRow();
  toBottom();
  if (state.watching && !reader.touching) addWaiting(state.watching);
}
function holdBar()       {
  readerActs();
  if (!reader.active) return;
  reader.holding = true;
  clearTimeout(reader.still);
  reader.still = setTimeout(letGoBar, stillMs);
}
function letGoBar()       {
  if (!reader.holding) return;
  reader.holding = false;
  clearTimeout(reader.still);
  readerActs();
}
const scrollKeys = new Set(["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " "]);
// Scrolling the list beside the conversation is not reading it.
const inSide = (e       )          => e.target instanceof Element && !!e.target.closest(".side");
for (const type of ["wheel", "touchstart", "touchmove"]) addEventListener(type, (e) => { if (!inSide(e)) readerActs(); }, { passive: true });
/** A finger on the glass: the page may bounce or glide on after it lifts, so the round goes on from the lift. A render
 *  may put a new node in place of the one under a finger (a reply comes in). Every touch event of that finger still
 *  goes to the node it first touched, out of the page now, and never reaches the window: that node hears the moves and
 *  the lift too. Each node a finger went down on keeps listening until no finger is down (two fingers: one node may be
 *  out of the page while the other lifts first). The next touchstart, or the page hidden (`cancelRound`), sets the
 *  finger again from what is down. */
const touched = new Set             ();
const fingerTypes = ["touchmove", "touchend", "touchcancel"];
function onTouched(e       )       {
  if ((e.currentTarget        ).isConnected) return;   // the window hears it
  if (e.type === "touchmove") readerActs(); else lift(e);
}
function letGoTouched()       {
  for (const at of touched) for (const type of fingerTypes) at.removeEventListener(type, onTouched);
  touched.clear();
}
function lift(e       )       {
  reader.touching = (e              ).touches.length > 0;
  if (reader.touching) return;
  letGoTouched();
  readerActs();
  // Older rows that came while the finger was down go in now, as the page glides (the glide keeps the round going, so
  // the round's end would put them in only at the top). At the top they still wait for the round's end.
  if (state.watching) addWaiting(state.watching);
}
addEventListener("touchstart", (e) => {
  const t = e              , at = e.target;
  if (t.touches.length === t.changedTouches.length) letGoTouched();   // a new gesture: the last one's lift went unheard
  if (at instanceof Node && !touched.has(at)) {
    for (const type of fingerTypes) at.addEventListener(type, onTouched, { passive: true });
    touched.add(at);
  }
  reader.touching = t.touches.length > 0;
}, { passive: true });
for (const type of ["touchend", "touchcancel"]) addEventListener(type, lift, { passive: true });
/** Pushing down at the bottom (or End, ⌘↓: to the bottom from anywhere) counts as reaching it, though nothing moves. */
const downKeys = new Set(["PageDown", "ArrowDown", " "]);
addEventListener("keydown", (e) => {
  if (!scrollKeys.has(e.key) || e.target instanceof HTMLTextAreaElement || e.target instanceof HTMLInputElement) return;
  readerActs();
  if (!reader.active) return;
  if (e.key === "End" || (e.key === "ArrowDown" && e.metaKey) || (downKeys.has(e.key) && nearBottom())) reachEnd();
});
addEventListener("wheel", (e) => { if (reader.active && e.deltaY > 0 && nearBottom()) reachEnd(); }, { passive: true });
addEventListener("pointerdown", (e) => { if (e.target === document.documentElement) holdBar(); });
addEventListener("pointermove", () => { if (reader.holding) holdBar(); }, { passive: true });
for (const type of ["pointerup", "pointercancel", "lostpointercapture", "blur"]) addEventListener(type, letGoBar);
addEventListener("pagehide", cancelRound);
document.addEventListener("visibilitychange", () => { if (document.visibilityState === "hidden") cancelRound(); });
/** A scroll during a round keeps the round going (until the page stops); any other scroll changes nothing. */
function scrolled()       {
  if (!reader.active) return;
  if (reader.holding) holdBar(); else readerActs();
}
addEventListener("scroll", scrolled, { passive: true });
window.visualViewport?.addEventListener("scroll", scrolled);
window.visualViewport?.addEventListener("resize", toBottom);
new ResizeObserver(toBottom).observe(document.body);
document.addEventListener("visibilitychange", () => { if (document.visibilityState === "visible") toBottom(); });
addEventListener("pageshow", toBottom);
function open(pane        )       { location.hash = `pane=${encodeURIComponent(pane)}`; }

// --- Helpers.
const order                         = { blocked: 0, working: 1, done: 2, idle: 3, unknown: 4 };
const statusText                         = { blocked: "Needs you", working: "Working", done: "Done", idle: "Idle", unknown: "Unknown" };
function agentPanes(ws             )                                      {
  return ws.flatMap((w) => w.tabs.flatMap((t) => t.panes.filter((p) => p.agent).map((p) => ({ w, t, p }))));
}
function blockedPanes(ws             )         { return agentPanes(ws).filter((x) => x.p.agent?.status === "blocked").map((x) => x.p); }
function agentName(p      )         { return p.agent ? `${p.agent.name} · ${p.agent.title || p.label || p.id}` : p.label || p.id; }
function find(pane        )                                                {
  for (const w of state.workspaces) for (const t of w.tabs) for (const p of t.panes) if (p.id === pane) return { w, t, p };
  return undefined;
}
// The app's StatusGlyph shapes: needs you = orange disc with "!", done = green disc with a check, working = faint ring
// with a blue arc (turns), idle = hollow ring.
const glyphs                         = {
  blocked: '<circle cx="8" cy="8" r="8" class="g-needs"/><path d="M8 4.2v4.6" stroke="#111" stroke-width="2" stroke-linecap="round"/><circle cx="8" cy="11.6" r="1.15" fill="#111"/>',
  done: '<circle cx="8" cy="8" r="8" class="g-done"/><path d="M4.6 8.3l2.2 2.2 4.6-4.8" fill="none" stroke="#111" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>',
  working: '<circle cx="8" cy="8" r="6" fill="none" class="g-ring" stroke-width="2"/><circle cx="8" cy="8" r="6" fill="none" class="g-arc" stroke-width="2" stroke-linecap="round" stroke-dasharray="11.3 100" transform="rotate(-90 8 8)"/>',
  idle: '<circle cx="8" cy="8" r="6.6" fill="none" class="g-idle" stroke-width="1.3"/>',
  unknown: '<circle cx="8" cy="8" r="6.6" fill="none" class="g-idle" stroke-width="1.3" stroke-dasharray="2 2"/>',
};
function dot(s        )              {
  const el = h("span", { class: `dot ${s}`, role: "img", "aria-label": statusText[s] });
  el.innerHTML = `<svg viewBox="0 0 16 16" aria-hidden="true">${glyphs[s]}</svg>`;   // fixed markup, no data
  return el;
}
const canInput = ()          => state.level === "input";
function toast(text        )       {
  state.toast = text;
  render();
  setTimeout(() => { if (state.toast === text) { state.toast = null; render(); } }, 4000);
}
function explain(code         , message         )         {
  switch (code) {
    case "stale": return "The prompt changed on the Mac. Nothing was sent.";
    case "promptOpen": return "Answer the open prompt first.";
    case "readOnly": return "This device can only read. Turn on input for it on the Mac.";
    case "notFound": return "That agent is gone.";
    case "rateLimited": return "Too many images from this phone. Try again in a few minutes.";
    default: return message || "It did not work.";
  }
}

// --- Screens.
// --- Wide screens (an iPad, 768 pt and up): the agents on the left, the conversation on the right; choosing an agent
// changes only the right column. The reader may hide the list while an agent is open. Narrower (a phone, Split View at a
// third): one screen at a time, the list or a conversation (`wideScreen`).
function split()          { return !!state.token && state.welcomed && wideScreen.matches; }
function sideShown()          { return split() && (!state.watching || !state.sideHidden); }
function toggleSide()       {
  state.sideHidden = !state.sideHidden;
  save(SIDE_KEY, state.sideHidden ? "hidden" : null);
  keepTopRow = !following;   // the right column gets wider or narrower: the row being read stays in place
  render();
}
/** The list beside the conversation: one node that stays in the page while shown (a screen reader does not hear it
 *  again on each tick: the page is `aria-live`), and where it was scrolled to, kept while it is hidden. */
const side = h("aside", { id: "agents", class: "side", "aria-label": "Agents", "aria-live": "off" });
let sideScroll = 0;
side.addEventListener("scroll", () => { if (side.isConnected) sideScroll = side.scrollTop; }, { passive: true });

function render()       {
  const anchor = keepTopRow ? topRow() : null;
  keepTopRow = false;
  const focused = document.activeElement?.id;   // read first: the list's rows are made anew below
  const count = blockedPanes(state.workspaces).length;
  document.title = count ? `(${count}) Flockbook` : "Flockbook";
  const body          = [];
  const wide = split(), showSide = sideShown();
  app.classList.toggle("split", wide);
  app.classList.toggle("side-shown", showSide);
  // The list on the left stays in the page (only its rows are made anew) and where it was scrolled to.
  const sideWasIn = side.isConnected;
  if (!state.token) body.push(pairScreen());
  else if (!state.welcomed) body.push(h("p", { class: "center muted" }, state.connected ? "Connecting…" : "Not connected. Trying again…"));
  else if (wide) {
    if (showSide) {
      side.replaceChildren(listScreen());
      body.push(side);
    }
    body.push(state.watching ? conversationScreen(state.watching) : h("p", { class: "center muted" }, "Choose an agent."));
  }
  else if (state.watching) body.push(conversationScreen(state.watching));
  else body.push(listScreen());
  if (state.banner && state.token) {
    const b = state.banner;
    body.push(h("button", { class: "banner glass-float", onclick: () => { state.banner = null; open(b.pane); } }, dot("blocked"), b.text));
  }
  if (state.toast) body.push(h("div", { class: "toast", role: "status" }, state.toast));
  const nodes = body.filter((c)                     => !!c);
  // Keep each agent's composer text (its own) across renders.
  const old = document.getElementById("composer-text")                              ;
  if (old && state.composerPane) state.drafts[state.composerPane] = old.value;
  const kept = keepScreen(nodes);
  // Nodes already in their place stay (the list on the left; see `syncChildren`).
  if (!kept) { fillItems(nodes); syncChildren(app, nodes); }
  if (side.isConnected && !sideWasIn) side.scrollTop = sideScroll;
  if (!kept?.composer) {
    // Another screen, pane or composer: that pane's own draft comes back (not the text of the composer before).
    const fresh = document.getElementById("composer-text")                              ;
    if (fresh && state.watching) { fresh.value = state.drafts[state.watching] ?? ""; fitText(fresh); }
  }
  const box = document.getElementById("composer-text")                              ;
  state.composerPane = box ? state.watching : null;
  // Pulled texts of this agent go after what the composer holds, each on its own line; then the hub may forget them.
  const fills = state.watching ? state.fills[state.watching] : undefined;
  if (box && fills?.length) {
    const warnings           = [];
    for (const f of fills) {
      box.value = box.value ? `${box.value}\n${f.text}` : f.text;
      if (f.id) { state.placed.add(f.id); ack(f.id); }
      if (f.note) warnings.push(f.note);
      else if (f.interrupted) warnings.push("The agent's turn stopped after the pull.");
    }
    if (warnings.length) queueMicrotask(() => toast(`Your text is in the composer. ${[...new Set(warnings)].join(" ")}`));
    delete state.fills[state.watching ];
    box.dispatchEvent(new Event("input"));
    box.focus({ preventScroll: true });
  }
  if (focused && document.activeElement !== document.getElementById(focused)) document.getElementById(focused)?.focus({ preventScroll: true });
  // Older rows came in above, or the column changed width: the row that was on top stays where it was on screen.
  if (anchor) putBack(anchor);
  noteSeenRow();
  dropPictures();
  toBottom();
  morphGlyphs();
  cardIn();
}

// --- Morphs. Opening an agent, its row turns into the bar and its glyph and title fly into it (View Transitions);
// without them (Safari before 18) the conversation opens out of the row's box. A status glyph that changes pops into
// its new shape; a new prompt card rises in. Reduce Motion: none of it, the screen just changes.
const still = matchMedia("(prefers-reduced-motion: reduce)");
const EASE = "cubic-bezier(.2, .85, .2, 1)", SPRING = "cubic-bezier(.34, 1.4, .64, 1)";
                                                                                                       
let morphing = false;   // one transition at a time: a render during one just happens
                                                                                             
function rowParts(pane        )                     {
  const r = document.getElementById(`row-${pane}`);
  return r ? [r, r.querySelector(":scope > .dot"), r.querySelector(".row-title")] : [];
}
function barParts()                     {
  const b = app.querySelector(":scope > section.conversation > .top");
  return b ? [b, b.querySelector(".bar-title > .dot"), b.querySelector(".bar-title > span:last-child")] : [];
}
function named(parts                    , on         )       {
  ["fb-sheet", "fb-dot", "fb-title"].forEach((n, i) => (parts[i]                      )?.style.setProperty("view-transition-name", on ? n : ""));
}
/** Runs `update` (the render that changes screens), morphing from the row of the agent opened or closed. */
function navigate(from               , to               , update            )       {
  const pane = to ?? from;
  const before = pane && !still.matches && !split() && state.token && state.welcomed ? (to ? rowParts(pane) : barParts()) : [];
  if (!pane || !before.length || !before[0]) { update(); return; }
  const after = ()                     => (to ? barParts() : rowParts(pane));
  const doc = document                 ;
  if (doc.startViewTransition) {
    if (document.visibilityState !== "visible" || morphing) { update(); return; }
    morphing = true;
    named(before, true);
    const t = doc.startViewTransition(() => { named(before, false); update(); named(after(), true); });
    // A skipped or timed-out transition still ran (or runs) the update; only the animation is lost.
    t.ready.catch(() => {});
    t.updateCallbackDone.catch(() => {});
    t.finished.catch(() => {}).then(() => { morphing = false; named(after(), false); });
    return;
  }
  const box = before[0].getBoundingClientRect();
  update();
  const section = app.querySelector             (":scope > section.conversation");
  if (to && section?.animate) {
    const r = section.getBoundingClientRect();
    const start = `inset(${box.top - r.top}px ${r.right - box.right}px ${r.bottom - box.bottom}px ${box.left - r.left}px round 18px)`;
    section.animate([{ clipPath: start, opacity: .6 }, { clipPath: "inset(0px 0px 0px 0px round 0px)", opacity: 1 }], { duration: 420, easing: EASE });
  } else if (!to) {
    document.getElementById(`row-${pane}`)?.animate?.([{ transform: "scale(1.04)", opacity: .5 }, { transform: "none", opacity: 1 }], { duration: 380, easing: EASE });
  }
}
/** The last glyph seen for each row and bar: a changed one pops into its new shape. */
const glyphSeen = new Map                ();
function morphGlyphs()       {
  for (const d of app.querySelectorAll             (".row[data-pane] > .dot, section.conversation > .top .bar-title > .dot")) {
    const row = d.closest             (".row"), key = row ? `row:${row.dataset.pane}` : `bar:${d.closest             ("section")?.dataset.pane}`;
    const s = d.classList[1], was = glyphSeen.get(key);
    glyphSeen.set(key, s);
    if (was && was !== s && !still.matches && d.animate) {
      d.animate([{ transform: "scale(.2) rotate(-90deg)", opacity: 0 }, { transform: "scale(1.2)", opacity: 1, offset: .6 }, { transform: "none", opacity: 1 }],
        { duration: 520, easing: EASE });
    }
  }
}
/** Each prompt card rises in once, the first time it shows (cards are made anew on each render). */
const cardsShown = new Set        ();
function cardIn()       {
  const c = app.querySelector             (":scope > section.conversation > .card[data-prompt]");
  const key = c?.dataset.prompt;
  if (!c || !key || cardsShown.has(key)) return;
  cardsShown.add(key);
  if (!still.matches && c.animate) c.animate([{ transform: "translateY(14px) scale(.97)", opacity: 0 }, { transform: "none", opacity: 1 }], { duration: 360, easing: SPRING });
}
function putBack(anchor                             )       {
  const row = [...document.querySelectorAll             (".items > [data-id]")].find((e) => e.dataset.id === anchor.id);
  const moved = row ? row.getBoundingClientRect().top - anchor.top : 0;
  if (Math.abs(moved) >= 1) {
    window.scrollBy(0, moved);
    // Not the reader's move: the round goes on from the same rows.
    reader.from += moved; reader.last += moved; reader.endTop += moved;
  }
}
/** The row a reader who scrolled up was reading, noted after each render and each round. A turn or a resize lays the
 *  rows out again before the page hears of it: the row goes back to where it was then. */
let seenRow                                     = null;
function noteSeenRow()       { seenRow = state.watching && !following ? topRow() : null; }
addEventListener("resize", () => { if (seenRow && state.watching && !following && !reader.active) putBack(seenRow); });

// --- Pictures (remote-protocol.md 3.4). The hub sends none with the conversation: a picture is asked for when it comes
// near the view and let go when it leaves. Kept in memory only (blob URLs), never in a cache or storage.
const PICTURE_HEIGHT = 420;
/** Parts of the `picture` requests on the way, by request id. */
const pictureParts = new Map                                                        ();
function requestPicture(pane        , item        , index        , full         )                       {
  const id = `c${nextId++}`;
  pictureParts.set(id, { parts: [], got: 0, mime: "" });
  return request({ type: "picture", pane, item, index, full }, id).then((r) => {
    const p = pictureParts.get(id);
    pictureParts.delete(id);
    if (!r.ok || !p || !p.parts.length || p.got !== p.parts.length) return null;
    try { return new Blob(p.parts.map((x) => Uint8Array.from(atob(x), (c) => c.charCodeAt(0))), { type: p.mime }); } catch { return null; }
  });
}

/** Picture boxes watched for the view, those near it, and the blob URL each one shows. */
const observed = new Set             ();
const near = new Set             ();
const shownURLs = new Map                     ();
const seen = new IntersectionObserver((entries) => {
  for (const e of entries) {
    const b = e.target               ;
    if (!observed.has(b) || !b.isConnected) continue;   // an entry queued before the box left the page
    if (e.isIntersecting) { near.add(b); showPicture(b); } else { near.delete(b); hidePicture(b); }
  }
}, { rootMargin: "600px 0px" });
function observePicture(b             )       { observed.add(b); seen.observe(b); }

/** The pictures of an item: each keeps its place (its size in the log, at most the column wide and `PICTURE_HEIGHT` high). */
function picturesView(pane        , item        , list           )              {
  return h("div", { class: "pictures" }, ...list.map((p, index) => {
    const w = Math.max(1, p.width), ht = Math.max(1, p.height);
    const b = h("button", { type: "button", class: "picture", "aria-label": `Picture ${index + 1}, open`, onclick: () => openViewer(b, w, ht) });
    Object.assign(b.dataset, { pane, item, index: String(index) });
    // CSSOM, not a style attribute (the page's CSP has no inline styles).
    b.style.aspectRatio = `${w} / ${ht}`;
    b.style.width = `min(${w}px, 100%, ${Math.round((PICTURE_HEIGHT * w) / ht)}px)`;
    observePicture(b);
    return b;
  }));
}

function showPicture(b             )       {
  if (shownURLs.has(b) || b.dataset.loading) return;
  b.dataset.loading = "1";
  void requestPicture(b.dataset.pane , b.dataset.item , Number(b.dataset.index), false).then((blob) => {
    delete b.dataset.loading;
    // Left the view (or the page) meanwhile: not shown; it is asked for again when it comes back.
    if (!blob || !near.has(b) || !b.isConnected || shownURLs.has(b)) return;
    const url = URL.createObjectURL(blob);
    shownURLs.set(b, url);
    b.replaceChildren(h("img", { src: url, alt: "" }));
  });
}
function hidePicture(b             )       {
  const url = shownURLs.get(b);
  if (url === undefined) return;
  shownURLs.delete(b);
  b.replaceChildren();
  URL.revokeObjectURL(url);
}
/** After a render: pictures out of the page let go of their URLs and are not watched (a row put back watches them again).
 *  The viewer closes when its picture left the conversation: another pane or session (in the conversation or in the
 *  snapshot: an ended session or a removed pane sends no new conversation), or the item without it. */
function dropPictures()       {
  for (const b of observed) if (!b.isConnected) { observed.delete(b); near.delete(b); hidePicture(b); seen.unobserve(b); }
  if (!viewer) return;
  const v = view(viewer.pane), it = v.items.find((i) => i.id === viewer .item);
  const images = it && "images" in it ? it.images : undefined;
  const now = find(viewer.pane)?.p.agent?.session;
  if (state.watching !== viewer.pane || v.session !== viewer.session || now !== viewer.session || (images?.length ?? 0) <= viewer.index) closeViewer();
}

/** The full picture over the page. It shows the preview until the full one comes; tap it for its own size. */
let viewer                                                                                                                                   = null;
function openViewer(b             , w        , ht        )       {
  closeViewer();
  const img = h("img", { alt: "", width: w, height: ht, onclick: () => img.classList.toggle("zoomed") });
  const preview = shownURLs.get(b);
  if (preview) img.src = preview;
  const status = h("p", { class: "viewer-status", role: "status" }, "Loading…");
  const node = h("div", { class: "viewer", role: "dialog", "aria-modal": "true", "aria-label": "Picture" },
    h("button", { type: "button", class: "viewer-close glass-float", onclick: closeViewer }, "Close"), status, img);
  const pane = b.dataset.pane , item = b.dataset.item , index = Number(b.dataset.index);
  const mine = { node, url: null                 , from: b, pane, session: view(pane).session, item, index };
  viewer = mine;
  dragToClose(node, img, mine);
  document.body.append(node);
  (node.querySelector(".viewer-close")               ).focus();
  void requestPicture(pane, item, index, true).then((blob) => {
    if (viewer !== mine) return;
    if (!blob) { status.textContent = preview ? "Could not load the full picture." : "Could not load the picture."; return; }
    mine.url = URL.createObjectURL(blob);
    img.src = mine.url;
    status.remove();
  });
}
/** Dragged up or down like Photos: the picture follows one finger and the page shows through. Let go past a quarter
 *  of the screen, or flicked that way, it slides out and closes; else it springs back. Zoomed in, sideways or with a
 *  second finger it does nothing (a second finger mid-drag springs it back). Reduced motion: no slide, no spring. */
function dragToClose(node             , img             , mine                            )       {
  let start                                  = null, dragging = false, dy = 0, trail                             = [];
  // Each slide (spring back or close) and each new drag starts a round; a slide's end from an older round does nothing.
  let round = 0;
  const zoomed = () => img.classList.contains("zoomed") || Math.abs((window.visualViewport?.scale ?? 1) - 1) > 0.01;
  const still = () => matchMedia("(prefers-reduced-motion: reduce)").matches;
  const place = (y        ) => {
    img.style.transform = y ? `translateY(${y}px)` : "";
    node.style.setProperty("--drag", String(Math.min(1, Math.abs(y) / (innerHeight / 2))));
  };
  // Waits for the slide, with a timer in case no transitionend comes (nothing moved, or the tab was hidden).
  const settle = (done            ) => {
    let over = false;
    const mine = ++round;
    const end = () => { if (over) return; over = true; img.removeEventListener("transitionend", end); if (round === mine) done(); };
    img.addEventListener("transitionend", end);
    setTimeout(end, 350);
  };
  const back = () => {
    dragging = false; start = null;
    if (still()) { place(0); node.classList.remove("moving"); return; }
    node.classList.add("settling");
    place(0);
    settle(() => node.classList.remove("settling", "moving"));
  };
  const close = (down         ) => {
    dragging = false; start = null;
    if (still()) { closeViewer(); return; }
    node.classList.add("settling", "closing");
    img.style.transform = `translateY(${down ? innerHeight : -innerHeight}px)`;
    node.style.setProperty("--drag", "1");
    settle(() => { if (viewer === mine) closeViewer(); });
  };
  node.addEventListener("touchstart", (e) => {
    if (node.classList.contains("closing")) return;
    if (e.touches.length !== 1) { if (dragging) back(); start = null; return; }
    if (zoomed()) return;
    start = { x: e.touches[0].clientX, y: e.touches[0].clientY };
    trail = [{ t: e.timeStamp, y: start.y }];
  }, { passive: true });
  node.addEventListener("touchmove", (e) => {
    if (!start) return;
    if (e.touches.length !== 1) { if (dragging) back(); start = null; return; }
    const t = e.touches[0], dx = t.clientX - start.x;
    dy = t.clientY - start.y;
    if (!dragging) {
      // Under the threshold it may still be a tap: cancelling the move would drop its click.
      if (Math.hypot(dx, dy) < 10) return;
      if (Math.abs(dy) <= Math.abs(dx)) { start = null; return; }
      dragging = true;
      round++;
      node.classList.remove("settling");
      node.classList.add("moving");
    }
    // A drag: the page under the picture does not move with it.
    e.preventDefault();
    place(dy);
    trail.push({ t: e.timeStamp, y: t.clientY });
    while (trail.length > 2 && e.timeStamp - trail[0].t > 150) trail.shift();
  }, { passive: false });
  node.addEventListener("touchend", (e) => {
    if (!dragging) { start = null; return; }
    // Speed over the last 100 ms; a finger that stopped before lifting has none.
    const from = trail.find((p) => e.timeStamp - p.t <= 100), last = trail[trail.length - 1];
    const v = from && e.timeStamp > from.t ? (last.y - from.y) / (e.timeStamp - from.t) : 0;
    if (Math.abs(dy) > innerHeight / 4 || (Math.abs(v) > 0.5 && Math.sign(v) === Math.sign(dy))) close(dy > 0);
    else back();
  });
  node.addEventListener("touchcancel", () => { if (dragging) back(); start = null; });
}
function closeViewer()       {
  if (!viewer) return;
  viewer.node.remove();
  if (viewer.url) URL.revokeObjectURL(viewer.url);
  // Back to the box it opened from, or the box that shows the same picture now (rows are built again on changes).
  const v = viewer;
  const from = v.from.isConnected ? v.from
    : [...document.querySelectorAll             ("button.picture")].find((b) => b.dataset.pane === v.pane && b.dataset.item === v.item && b.dataset.index === String(v.index));
  from?.focus({ preventScroll: true });
  viewer = null;
}
addEventListener("keydown", (e) => { if (e.key === "Escape" && viewer) closeViewer(); });
addEventListener("hashchange", closeViewer);

// --- Older rows. The reader scrolling near the top loads them; "Load earlier" stays for a keyboard or screen reader,
// and to try again after a failure. Auto-loading stops at `AUTO_ITEMS` rows in a pane (the button still loads more).
const NEAR_TOP = 600, AUTO_ITEMS = 1000, EARLIER_NOTICE_MS = 500, EARLIER_BATCH = 12;
/** A fast scroll up starts the read farther away: as far as the page goes, at its speed, in the time older rows take to
 *  come and go in (with half again to spare), so they are in before the top. That time is measured on each read. */
let earlierMs = 400;
/** When the last read began, and how long its rows waited at the top (not the read's own time). */
let earlierAsked = 0, earlierWaited = 0, waitSince = 0;
const lead = ()         => Math.max(NEAR_TOP, (reader.speed * earlierMs * 1.5) / 1000);
/** Nearer the top than this while the reader scrolls (the bounce there, a push into it) older rows wait, and while a
 *  finger is on the glass; farther down they go in while the page glides. */
const EDGE = 1;
/** The next render follows a prepend: it keeps the top row in place. */
let keepTopRow = false;
/** The last read of older rows: its pane, session and number. Another pane or session does not see it, and a reply to
 *  an older read changes nothing. */
let earlier = { pane: "", session: "", gen: 0, busy: false, failed: false, noticeUntil: 0 };
const earlierOf = (pane        )                        => (earlier.pane === pane && earlier.session === view(pane).session ? earlier : null);
/** The first row at least partly in view, and where it is. Read before the new rows are built (they move the old
 *  row nodes). */
function topRow()                                     {
  for (const e of document.querySelectorAll             (".items > [data-id]")) {
    const r = e.getBoundingClientRect();
    if (r.bottom > 0) return { id: e.dataset.id ?? "", top: r.top };
  }
  return null;
}
/** The next few older rows that came in go in front, the row on top staying in place; the rest a frame later. While the
 *  reader still scrolls the pane on screen (a finger on the glass, a glide, the bounce at the top) they wait, and go in
 *  when the round ends: a phone takes the page back to its own top after a bounce, whatever the page does meanwhile, so
 *  rows put in then push the message being read out of view. */
const addTimers = new Map                ();
function addWaiting(pane        )       {
  clearTimeout(addTimers.get(pane));   // one run per pane: new rows that come in while one runs join it
  addTimers.delete(pane);
  const v = view(pane);
  if (!v.waiting?.length) { if (v.waiting) { v.waiting = undefined; render(); } return; }
  if (state.watching === pane && (reader.touching || (reader.active && viewTop() < EDGE))) {
    if (!waitSince) waitSince = performance.now();
    render(); return;
  }
  if (waitSince) { earlierWaited += performance.now() - waitSince; waitSince = 0; }
  v.items = [...v.waiting.splice(Math.max(0, v.waiting.length - EARLIER_BATCH)), ...v.items];
  if (!v.waiting.length) {
    v.waiting = undefined;
    if (earlierAsked) { earlierMs = Math.min(2000, Math.max(200, 0.7 * earlierMs + 0.3 * (performance.now() - earlierAsked - earlierWaited))); earlierAsked = 0; }
  }
  keepTopRow = state.watching === pane;
  render();
  if (v.waiting) { const session = v.session; addTimers.set(pane, setTimeout(() => { if (view(pane).session === session) addWaiting(pane); }, 16)); }
}
async function loadEarlier(pane        )                {
  const v = view(pane), session = v.session;
  if (!session || !v.hasEarlier || !v.items.length || earlierOf(pane)?.busy) return;
  earlierAsked = performance.now(); earlierWaited = 0;
  const gen = earlier.gen + 1;
  earlier = { pane, session, gen, busy: true, failed: false, noticeUntil: performance.now() + EARLIER_NOTICE_MS };
  // The notice and the button above the rows change: the row on top stays in place (the page has no scroll anchoring).
  keepTopRow = state.watching === pane;
  render();
  const r = await request({ type: "loadEarlier", session, before: (v.waiting?.[0] ?? v.items[0]).id });
  if (earlier.gen !== gen) return;
  // The notice stays half a second at least (a flash reads as a glitch); the read itself is over.
  const wait = earlier.noticeUntil - performance.now();
  if (wait > 0) setTimeout(render, wait);
  Object.assign(earlier, { busy: false, failed: !r.ok });
  keepTopRow = state.watching === pane;
  render();
}
/** Only the reader scrolling up loads: a script's scroll (opening, switching, a page that got shorter) does not. */
function nearTop()       {
  const pane = state.watching;
  if (!pane || !state.welcomed || !reader.active || viewTop() >= reader.from || window.scrollY > lead()) return;
  const v = view(pane);
  if (v.waiting || v.items.length >= AUTO_ITEMS || earlierOf(pane)?.failed) return;
  loadEarlier(pane);
}
addEventListener("scroll", nearTop, { passive: true });

/** Row nodes by pane and item id, with the item they show: a tick that leaves a row as it was keeps its node (no
 *  markdown parsed again, an open tool list stays open). */
let rowNodes = new Map                                             ();
let rowPane = "";
function rowsFor(pane        , list        )                {
  const old = rowPane === pane ? rowNodes : new Map                                             ();
  rowNodes = new Map();
  rowPane = pane;
  return list.map((it) => {
    const item = JSON.stringify(it);
    const had = old.get(it.id);
    const node = had && had.item === item ? had.node : itemView(it, pane);
    node.dataset.id = it.id;
    // A kept row that was out of the page (another screen) watches its pictures again.
    for (const b of node.querySelectorAll             ("button.picture")) observePicture(b);
    rowNodes.set(it.id, { item, node });
    return node;
  });
}

/** A new screen's rows' list and the rows that go in it. */
const itemsOf = new WeakMap                  ();
const rowsIn = (list         )                    => (itemsOf.get(list) ?? []).filter((c)                     => c !== null && c !== undefined && c !== false);
/** A screen that is not kept: its rows go in its list. */
function fillItems(nodes                   )       {
  for (const n of nodes) if (n instanceof Element) for (const l of n.querySelectorAll(".items")) if (itemsOf.has(l)) { l.append(...rowsIn(l)); itemsOf.delete(l); }
}
/** Makes `parent`'s children `desired`, in order, moving only what changed: a node already in its place stays in the
 *  page. A node taken out and put back loses its layout, and a page that put every row back on each render laid out
 *  every row again (a long stall when older rows came in). */
function syncChildren(parent      , desired                   )       {
  const want = desired.map((n) => typeof n === "string" ? document.createTextNode(n) : n);
  const keep = new Set(want);
  let cur = parent.firstChild;
  for (const n of want) {
    while (cur && !keep.has(cur)) { const next = cur.nextSibling; parent.removeChild(cur); cur = next; }
    if (cur === n) { cur = cur.nextSibling; continue; }
    parent.insertBefore(n, cur);
  }
  while (cur) { const next = cur.nextSibling; parent.removeChild(cur); cur = next; }
}

/** The same pane's conversation stays in the page across renders: its rows' list and the rows in it stay where they
 *  are (only new and changed rows go in), and so does its composer (see `keepComposer`). Null: another screen or pane,
 *  nothing done. */
function keepScreen(nodes                   )                               {
  const oldSection = app.querySelector             (":scope > section.conversation");
  const newSection = nodes.find((n)                   => n instanceof HTMLElement && n.matches("section.conversation"));
  if (!oldSection || !newSection || oldSection.dataset.pane !== newSection.dataset.pane) return null;
  const swap = new Map            ([[newSection, oldSection]]);
  const oldItems = oldSection.querySelector(":scope > .items"), newItems = newSection.querySelector(":scope > .items");
  if (oldItems && newItems) { syncChildren(oldItems, rowsIn(newItems)); swap.set(newItems, oldItems); }
  else fillItems([newSection]);
  const form = keepComposer(oldSection, newSection);
  if (form) swap.set(form.fresh, form.old);
  // A read-only device's status line at the foot stays while it is the same (its sideways scroll too).
  const oldFoot = oldSection.querySelector             (":scope > .status-foot"), newFoot = newSection.querySelector             (":scope > .status-foot");
  if (oldFoot && newFoot && oldFoot.querySelector             (".statusline")?.dataset.key === newFoot.querySelector             (".statusline")?.dataset.key) swap.set(newFoot, oldFoot);
  syncChildren(oldSection, [...newSection.childNodes].map((n) => swap.get(n) ?? n));
  syncChildren(app, nodes.map((n) => (typeof n === "string" ? n : swap.get(n) ?? n)));
  return { composer: !!form };
}

/** The hub sends news every tick (a compacting or working agent's screen changes each time). The same pane's composer
 *  stays in the page and only its controls are swapped: a phone keyboard closes when its text field leaves the page,
 *  and focusing a new one from script does not bring it back. Its text, caret, focus and IME input stay. Null: no
 *  composer for the same pane before and after, nothing done. */
function keepComposer(oldSection             , newSection             )                                                          {
  const oldForm = oldSection.querySelector                 (":scope > form.composer");
  const newForm = newSection.querySelector                 (":scope > form.composer");
  if (!oldForm || !newForm || oldForm.dataset.pane !== newForm.dataset.pane) return null;
  const oldRow = oldForm.querySelector(":scope > .composer-row"), newRow = newForm.querySelector(":scope > .composer-row");
  const oldBox = oldRow?.querySelector(":scope > textarea"), newBox = newRow?.querySelector(":scope > textarea");
  if (!oldRow || !newRow || !oldBox || !newBox) return null;
  // Everything around a kept element is replaced; the kept element itself never leaves the page.
  syncChildren(oldRow, [...newRow.childNodes].map((n) => (n === newBox ? oldBox : n)));
  // The status line stays while it is the same (its sideways scroll too).
  const oldLine = oldForm.querySelector             (":scope > .statusline"), newLine = newForm.querySelector             (":scope > .statusline");
  const sameLine = oldLine && newLine && oldLine.dataset.key === newLine.dataset.key;
  syncChildren(oldForm, [...newForm.childNodes].map((n) => (n === newRow ? oldRow : sameLine && n === newLine ? oldLine : n)));
  for (const a of ["placeholder", "aria-label"]) oldBox.setAttribute(a, newBox.getAttribute(a) ?? "");
  (oldBox                       ).disabled = (newBox                       ).disabled;
  return { old: oldForm, fresh: newForm };
}

function pairScreen()              {
  const name = h("input", { id: "pair-name", class: "field", value: defaultName(), maxlength: "40", autocomplete: "off", "aria-label": "Device name" });
  const words = state.pairSecret ? null
    : h("input", { id: "pair-words", class: "field", placeholder: "six words from the Mac", autocomplete: "off", autocapitalize: "none", spellcheck: "false", "aria-label": "Pairing words" });
  const go = async ()                => {
    const secret = state.pairSecret ?? words?.value.trim() ?? "";
    if (!secret || state.busy) return;
    state.busy = true;
    state.pairError = null;
    render();
    if (!ws || ws.readyState === WebSocket.CLOSING || ws.readyState === WebSocket.CLOSED) connect();
    await opened();
    const r = await request({ type: "pair", secret, name: name.value.trim() || defaultName() });
    state.busy = false;
    if (!r.ok) {
      state.pairSecret = null;
      state.pairError = r.code === "unauthorized" ? "That code is wrong or expired. Make a new one on the Mac." : explain(r.code, r.message);
    }
    render();
  };
  return h("section", { class: "pair" },
    h("h1", {}, "Pair with your Mac"),
    h("p", { class: "muted" }, state.pairSecret
      ? "This link came from the QR code on your Mac. Name this device, then pair."
      : "On the Mac: Settings › Remote access › Pair a Device. Type the six words it shows."),
    h("label", { class: "label", for: "pair-name" }, "Device name"), name,
    words && h("label", { class: "label", for: "pair-words" }, "Code"), words,
    h("button", { id: "pair-go", class: "primary", disabled: state.busy, onclick: go }, state.busy ? "Pairing…" : "Pair"),
    state.pairError && h("p", { class: "error", role: "alert" }, state.pairError),
    state.fingerprint && h("p", { class: "fine" }, `Certificate ${state.fingerprint.slice(0, 16)}… (for reference only: the browser checks the certificate, this page cannot)`),
    h("p", { class: "fine" }, "A paired device can read your agents' conversations. Input stays off until you turn it on for this device on the Mac."));
}

function defaultName()         {
  const ua = navigator.userAgent;
  return /iPhone/.test(ua) ? "iPhone" : /iPad/.test(ua) ? "iPad" : /Android/.test(ua) ? "Android phone" : "Browser";
}

function listScreen()              {
  const all = agentPanes(state.workspaces);
  const needs = all.filter((x) => x.p.agent?.status === "blocked");
  const row = (x                                   , showWs         )              =>
    h("li", {}, h("button", { id: `row-${x.p.id}`, class: "row glass-float", "data-pane": x.p.id, "aria-current": split() && x.p.id === state.watching ? "page" : null, onclick: () => open(x.p.id) },
      dot(x.p.agent .status),
      h("span", { class: "row-text" },
        h("span", { class: "row-title" }, x.p.agent .title || x.t.label),
        h("span", { class: "row-sub" }, [x.p.agent .name, showWs ? x.w.label : null, x.t.label].filter(Boolean).join(" · "))),
      h("span", { class: "chev", "aria-hidden": "true" }, "›")));
  const sections          = [];
  if (needs.length) {
    sections.push(h("h2", { id: "needs-you" }, `Needs you · ${needs.length}`), h("ul", { class: "group needs" }, ...needs.map((x) => row(x, true))));
  }
  for (const w of state.workspaces) {
    const rows = all.filter((x) => x.w.id === w.id && x.p.agent?.status !== "blocked")
      .sort((a, b) => order[a.p.agent .status] - order[b.p.agent .status] || a.t.number - b.t.number);
    if (!rows.length) continue;
    const counts = [[rows.filter((x) => x.p.agent .status === "working").length, "working"], [rows.filter((x) => x.p.agent .status !== "working").length, "resting"]]
      .filter(([n]) => n).map(([n, what]) => `${n} ${what}`).join(" · ");
    sections.push(h("h2", {}, h("span", { class: "mono" }, monogram(w.label)), w.label, h("span", { class: "parts" }, counts)), h("ul", { class: "group" }, ...rows.map((x) => row(x, false))));
  }
  if (!all.length) sections.push(h("p", { class: "center muted" }, "No agents running."));
  return h("section", { class: "list" },
    // A large title that scrolls away with the list (no bar on top: nothing on this page needs one).
    h("header", { class: "list-head" }, h("h1", {}, "Agents"),
      h("div", { class: "head-pills" },
        state.pushKey && (pushReady() || iosTab()) ? h("button", { id: "notify", class: `notify ${state.notify}`, type: "button",
          disabled: state.notify === "busy", "aria-pressed": state.notify === "on" ? "true" : "false", onclick: togglePush },
          state.notify === "on" ? "Notifications on" : "Notify me") : null,
        h("span", { class: `level ${state.level}` }, state.level === "input" ? "Input on" : "Read only"))),
    ...sections);
}

function conversationScreen(pane        )              {
  const where = find(pane);
  const v = view(pane);
  const title = where ? (where.p.agent?.title || where.t.label) : pane;
  const status = where?.p.agent?.status ?? "unknown";
  // Wide: the list stays beside the conversation; the button hides or shows it. Narrow: Back to the list.
  const shown = sideShown();
  // An agent team took the pane's log (the Mac's words): the lead's last known log says so under the title; with no
  // lead log known the pane waits for it and shows no rows (they would be another session's).
  const team = where?.p.agent?.team;
  const head = h("header", { class: "top bar glass-bar" },
    split()
      ? h("button", { id: "side-toggle", class: "back side-toggle", type: "button", "aria-controls": "agents", "aria-expanded": shown ? "true" : "false",
          "aria-label": shown ? "Hide agents" : "Show agents", title: shown ? "Hide agents" : "Show agents", onclick: toggleSide }, sidebarGlyph())
      : h("button", { class: "back", onclick: () => { location.hash = ""; } }, "‹ Agents"),
    h("span", { class: "bar-title" }, dot(status), team === "lastLead"
      ? h("span", { class: "bar-stack" }, h("span", {}, title), h("span", { class: "team-line" }, "Agent team: showing the lead's last known session."))
      : h("span", {}, title)));
  // Older rows on the way: a notice under the bar, over the rows (it moves none of them).
  const notice = earlierOf(pane);
  const noticed = !!v.waiting || !!(notice && (notice.busy || (!notice.failed && performance.now() < notice.noticeUntil)));
  const items          = [];
  const rows = team === "waitingForLead" ? [] : v.items;
  if (team === "waitingForLead") {
    items.push(h("div", { class: "team-wait center", role: "status" }, h("h2", {}, "Waiting for the lead's log"),
      h("p", { class: "muted" }, "Herdr points this pane at an agent-team teammate. The lead's conversation shows here once Herdr reports it. The terminal shows what it is doing.")));
  } else if (v.hasEarlier && v.session) {
    const mine = notice;
    if (mine?.failed) items.push(h("p", { class: "center muted", role: "status" }, "Could not load earlier messages."));
    // While the notice shows the button keeps its words and its room but is not seen (one "Loading earlier", not two).
    items.push(h("button", { id: "load-earlier", class: noticed ? "quiet under-notice" : "quiet", disabled: !!mine?.busy, "aria-busy": mine?.busy ? "true" : "false", onclick: () => loadEarlier(pane) },
      mine?.failed && !mine.busy ? "Try again" : "Load earlier"));
  }
  items.push(...rowsFor(pane, rows));
  if (!rows.length && !v.screen && team !== "waitingForLead") items.push(h("p", { class: "center muted" }, where ? "Loading…" : "This agent is gone."));
  if (v.screen) items.push(screenView(pane, v.screen, !rows.length || where?.p.agent === undefined));
  if (v.queue && status === "working") items.push(queueView(pane, v.queue));

  const card = v.approval ? approvalCard(pane, v.approval) : v.question ? questionCard(pane, v.question, v.items) : null;
  card?.setAttribute("data-prompt", `${pane}:${v.approval?.id ?? v.question?.id}`);
  const pill = noticed ? h("div", { class: "earlier-pill glass-float", role: "status" }, h("span", { class: "spin", "aria-hidden": "true" }), "Loading earlier…") : null;
  // The rows go in when the screen is put in the page (`keepScreen`, `fillItems`): putting the row nodes on hand in a
  // new list here would take them out of the page.
  const list = h("div", { class: "items" });
  itemsOf.set(list, items);
  const form = composer(pane, !!card, where?.p.agent?.name);
  // Claude Code's status line: in the composer, over the text field; a read-only device has it at the foot alone.
  const line = statusLineView(pane);
  if (form && line) form.prepend(line);
  return h("section", { class: "conversation", "data-pane": pane }, head, pill, list, card, form, form ? null : line && h("div", { class: "status-foot glass-float" }, line));
}

/** Claude Code's status line (appendices C and D): a row of tags over the composer's field (the model in bold, effort,
 *  context with its bar, orange from 80%, the cost marked as an estimate, then the owner's line cut into parts in their
 *  colours), only the ones that fit in one row, and a chevron at the right when one is left out. Open: every tag,
 *  wrapped, and when it was updated. Nothing when the hub sends none (the Mac's switch is off, or no line yet). */
function statusLineView(pane        )                     {
  const s = view(pane).statusLine;
  const tags = s ? statusTags(s) : [];
  if (!s || !tags.length) return null;
  const open = state.statusOpen.has(pane);
  const more = h("button", { class: "status-more", type: "button", "aria-expanded": open ? "true" : "false",
    "aria-label": open ? "Hide the whole status line" : "Show the whole status line", onclick: () => { if (open) state.statusOpen.delete(pane); else state.statusOpen.add(pane); render(); } },
    h("span", { class: `chev${open ? "" : " up"}`, "aria-hidden": "true" }));
  // Open, the row wraps and holds every tag.
  const row = h("div", { class: `status-tags${open ? " all" : ""}` }, ...tags);
  const head = h("div", { class: "status-head" }, row, more);
  // The chevron shows when a tag is left out of the row (wrapped onto the hidden second line).
  if (!open) {
    const fit = ()       => {
      // Measured without the chevron: with it, its own room could push the last tag out.
      more.hidden = true;
      const first = row.firstElementChild                      ;
      more.hidden = ![...row.children].some((t) => (t               ).offsetTop !== first?.offsetTop);
    };
    new ResizeObserver(fit).observe(row);
  }
  const time = s.time ? new Date(s.time * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }) : null;
  const body = open && time ? h("div", { class: "status-detail" }, h("div", { class: "status-time" }, `Updated ${time}`)) : null;
  return h("div", { class: `statusline${open ? " open" : ""}`, "data-key": JSON.stringify([s, open]) }, head, body);
}

/** One tag per fact Claude Code's JSON gives (none for a fact it did not send), then one per part of the owner's line. */
function statusTags(s            )                {
  const tag = (kind        , text        , spoken        , ...extra                          )              =>
    h("span", { class: `status-tag tag-${kind}`, role: "text", "aria-label": spoken }, text, ...extra);
  const out                = [];
  if (s.model) out.push(tag("model", s.model, `Model ${s.model}`));
  if (s.effort) out.push(tag("effort", s.effort, `Effort ${s.effort}`));
  if (s.contextPercent !== undefined) {
    const p = s.contextPercent;
    const fill = h("span", { class: "status-fill" });
    fill.style.width = `${Math.min(Math.max(p, 0), 100)}%`;
    const t = tag("context", `ctx ${p}%`, `Context ${p}% used`, h("span", { class: "status-bar", "aria-hidden": "true" }, fill));
    if (p >= 80) t.classList.add("warn");
    out.push(t);
  }
  if (s.costUSD !== undefined) {
    const d = `$${s.costUSD.toFixed(2)}`;
    out.push(tag("cost", `${d} est.`, `Session cost about ${d}, an estimate`));
  }
  for (const part of s.segments ?? []) {
    const text = part.map((r) => r.text).join("");
    out.push(h("span", { class: "status-tag tag-own", role: "text", "aria-label": text }, ...part.map(runView)));
  }
  return out;
}

/** One run in its colours (set through the style object: the page's policy allows no style attributes). A colour that
 *  does not read on the tag (under 4.5:1, StatusTag.readable) is left to the tag's own; dim text is a fixed grey. Light and dark are decided
 *  apart (--fg-light, --fg-dark; the stylesheet picks one), so a kept line follows the phone's switch. */
function runView(r           )              {
  const el = h("span", {}, r.text);
  const lum = (hex        )         => {
    const v = parseInt(hex.slice(1), 16);
    const lin = (c        )         => { const x = (c & 255) / 255; return x <= 0.04045 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4; };
    return 0.2126 * lin(v >> 16) + 0.7152 * lin(v >> 8) + 0.0722 * lin(v);
  };
  const reads = (hex        , dark         )          => {
    const [a, b] = [lum(hex), lum(dark ? "#2e2e2e" : "#f0f0f0")];
    return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05) >= 4.5;
  };
  // Dim text: a fixed grey that reads on the tag (StatusTag.dimHex), coloured or not.
  if (r.dim) {
    el.style.setProperty("--fg-light", "#6b6b6b");
    el.style.setProperty("--fg-dark", "#969696");
  } else if (r.fg && /^#[0-9a-f]{6}$/i.test(r.fg)) {
    if (reads(r.fg, false)) el.style.setProperty("--fg-light", r.fg);
    if (reads(r.fg, true)) el.style.setProperty("--fg-dark", r.fg);
  }
  if (r.bold) el.style.fontWeight = "700";
  if (r.italic) el.style.fontStyle = "italic";
  if (r.underline) el.style.textDecoration = "underline";
  return el;
}

/** A window with a column on its left (the sidebar symbol). */
function sidebarGlyph()                {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 24 20");
  svg.setAttribute("aria-hidden", "true");
  for (const [tag, attrs] of [["rect", { x: "1.5", y: "1.5", width: "21", height: "17", rx: "3.5" }], ["path", { d: "M9 1.5v17" }]]         ) {
    const e = document.createElementNS(ns, tag);
    for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
    svg.append(e);
  }
  return svg;
}

function itemView(it      , pane        )              {
  const images = "images" in it ? it.images : undefined;
  const pictures = images?.length ? picturesView(pane, it.id, images) : null;
  switch (it.kind) {
    case "user": return h("div", { class: "msg user" }, it.text ? h("p", {}, it.text) : null, pictures);
    case "assistant": return h("div", { class: "msg assistant" }, markdown(it.text));
    case "status": return h("p", { class: "status-line" }, it.text);
    case "tools":
      return h("div", { class: "tools" }, ...it.tools.map((c, i) => {
        const key = `${pane} ${it.id} ${i}`;
        return toggled(h("details", { class: "chip", open: state.chipsOpen.has(key) },
        h("summary", {}, c.text),
        h("ul", {}, ...c.calls.map((call) => h("li", { class: call.error ? "error" : call.done ? "" : "running" },
          h("span", { class: "tool" }, call.tool), " ", h("code", {}, call.summary))))),
        (open) => { if (open) state.chipsOpen.add(key); else state.chipsOpen.delete(key); });
      }), pictures);
    case "approval": {
      const a = it.approval;
      return h("div", { class: `logged ${a.state}` }, h("span", { class: "muted" }, a.state === "pending" ? "Asks: " : "Asked: "), a.title, " ", h("code", {}, a.command));
    }
    case "question": {
      const q = it.question;
      const said = q.state === "answered" && q.answers ? ` → ${q.answers.filter(Boolean).join("; ")}` : q.state === "declined" ? " → declined" : "";
      return h("div", { class: `logged ${q.state}` }, h("span", { class: "muted" }, "Question: "), q.questions.map((x) => x.text).join(" / "), said);
    }
  }
}

/** The agent's queued messages: faded user bubbles, when they go in, and Pull back (the hub sends Up or Shift-Left,
 *  never Escape, and checks the agent's screen before and after). */
function queueView(pane        , q       )              {
  const when = q.agent === "claude" ? "Goes in after the current step or when the turn ends." : "Goes in when this turn ends.";
  const note = q.pullsAll ? (q.items.length > 1 ? `All ${q.items.length} come back as one message.` : "It comes back to the composer.")
    : "Sending it again adds it to the turn now running.";
  // Only Claude Code's queue can be taken back exactly (its log); Codex's and Grok's are edited in their terminals.
  const edit = q.agent === "grok" ? "To change it, press Ctrl+; in Grok's terminal." : "To change it, press Shift+Left in Codex's terminal.";
  const blocked = !q.pullsAll ? `${when} ${edit}`
    : !q.pullable ? "The agent's input box has text. Clear it in the terminal to pull back."
    : q.items.includes(null) ? "The queued text cannot be read yet." : null;
  const pull = async ()                => {
    if (state.busy) return;
    state.busy = true;
    render();
    const r = await request({ type: "pullQueued", pane });
    state.busy = false;
    if (!r.ok) toast(explain(r.code, r.message));
    render();
  };
  return h("div", { class: "queue", role: "group", "aria-label": "Queued messages" },
    ...q.items.map((t) => h("div", { class: "msg user queued" }, h("span", { class: "queued-label" }, "Queued"),
      t === null ? h("p", { class: "unknown" }, "1 queued message") : h("p", {}, t))),
    h("div", { class: "queue-foot" }, h("span", { class: "fine" }, blocked ?? `${when} ${note}`),
      canInput() && q.pullsAll ? h("button", { id: "pull-queued", class: "secondary small", type: "button", disabled: state.busy || blocked !== null, onclick: pull },
        "Pull back to edit") : null));
}

function screenView(pane        , text        , byDefault         )              {
  return toggled(h("details", { class: "screen", open: state.screenOpen.get(pane) ?? byDefault }, h("summary", {}, "Screen"), h("pre", {}, text)),
    (open) => state.screenOpen.set(pane, open));
}

/** Calls `changed` when the reader opens or closes `e`. Built open, a `details` gets a toggle event of its own (the
 *  HTML spec queues one for any change of `open`): that one leaves `open` as built and is not the reader's. */
function toggled(e                    , changed                         )                     {
  let shown = e.open;
  e.addEventListener("toggle", () => { if (e.open !== shown) { shown = e.open; changed(e.open); } });
  return e;
}

function approvalCard(pane        , a          )              {
  const act = async (allow         )                => {
    if (state.busy) return;
    state.busy = true;
    render();
    const r = await request({ type: "approve", pane, prompt: a.id, allow });
    state.busy = false;
    if (!r.ok) toast(explain(r.code, r.message));
    render();
  };
  return h("div", { class: "card approval glass-float", role: "group", "aria-label": "Permission request" },
    h("div", { class: "card-head" }, h("span", { class: "ask", "aria-hidden": "true" }, "!"), h("strong", {}, a.title || `${a.tool} wants to run`)),
    a.command && h("pre", { class: "command" }, a.command),
    canInput()
      ? h("div", { class: "actions" },
        h("button", { id: "deny", class: "secondary", disabled: state.busy, onclick: () => act(false) }, "Deny"),
        h("button", { id: "allow", class: "primary", disabled: state.busy, onclick: () => act(true) }, "Allow"))
      : h("p", { class: "fine" }, "Read only: answer on the Mac, or turn on input for this device."));
}

/** The prompt shows the question on screen; Claude shows one of a set at a time. The log item has all of them. */
function fullQuestion(q                 , items        )                         {
  if (q.headers.length <= q.questions.length) return q;
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.kind !== "question" || it.question.state !== "pending") continue;
    const all = it.question.questions;
    if (all.length === q.headers.length && all.every((x, n) => x.header === q.headers[n])) return { ...q, questions: all };
  }
  return null;
}

function questionCard(pane        , prompt                 , items        )              {
  const q = fullQuestion(prompt, items);
  const head = h("div", { class: "card-head" }, h("span", { class: "ask", "aria-hidden": "true" }, "?"), h("strong", {}, "Question"));
  if (!q || q.delivery !== "picker" || q.onMac || !canInput()) {
    const why = !canInput() ? "Read only: answer on the Mac, or turn on input for this device."
      : !q ? `${prompt.headers.length} questions: answer them on the Mac.`
      : q.onMac ? "Answer this question on the Mac." : "Answer this one in the terminal.";
    return h("div", { class: "card question glass-float" }, head,
      ...prompt.questions.map((x) => h("p", {}, x.text)), h("p", { class: "fine" }, why));
  }
  const picks                = q.questions.map(() => new Set        ());
  const others                     = [];
  const submit = h("button", { id: "submit-answer", class: "primary", disabled: true }, "Send answer");
  const ready = ()          => q.questions.every((_, n) => picks[n].size > 0 || others[n].value.trim() !== "");
  const blocks = q.questions.map((x, n) => {
    const other = h("input", { class: "field other", placeholder: "Other…", autocomplete: "off", "aria-label": `Other answer to: ${x.text}` });
    other.addEventListener("input", () => { submit.disabled = !ready(); });
    others.push(other);
    const opts = x.options.map((o, i) => {
      const b = h("button", { class: "option", "aria-pressed": "false", onclick: () => {
        if (!x.multiSelect) { picks[n].clear(); for (const s of b.parentElement .children) s.setAttribute("aria-pressed", "false"); }
        picks[n].has(i) ? picks[n].delete(i) : picks[n].add(i);
        b.setAttribute("aria-pressed", String(picks[n].has(i)));
        submit.disabled = !ready();
      } }, h("span", { class: "option-label" }, inline(o.label), o.recommended ? h("span", { class: "rec" }, " Recommended") : null),
        o.detail ? h("span", { class: "option-detail" }, o.detail) : null);
      return b;
    });
    return h("fieldset", {}, h("legend", {}, x.header ? `${x.header}: ` : "", x.text),
      h("div", { class: "options" }, ...opts), other, x.multiSelect ? h("p", { class: "fine" }, "Pick one or more.") : null);
  });
  submit.addEventListener("click", async () => {
    if (state.busy || !ready()) return;
    const answers           = q.questions.map((_, n) => {
      const other = others[n].value.trim();
      return other ? { picks: [...picks[n]].sort(), other } : { picks: [...picks[n]].sort() };
    });
    state.busy = true;
    submit.disabled = true;
    const r = await request({ type: "answer", pane, question: q.id, answers });
    state.busy = false;
    if (!r.ok) toast(explain(r.code, r.message));
    render();
  });
  return h("div", { class: "card question glass-float", role: "group", "aria-label": "Question" }, head, ...blocks, h("div", { class: "actions" }, submit));
}

// --- Images (remote-protocol.md 3.3): picked on the phone, sent in parts with `attach`, named by `send`.
function readAsDataURL(f      )                  {
  return new Promise((resolve) => {
    const r = new FileReader();
    r.onload = () => resolve(typeof r.result === "string" ? r.result : "");
    r.onerror = () => resolve("");
    r.readAsDataURL(f);
  });
}

function base64(bytes            )         {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

/** Adds picked files: at most 4 per message, each at most 10 MB. The hub checks the type itself. */
async function pick(pane        , files        )                {
  const list = state.attachments.get(pane) ?? [];
  const problems           = [];
  for (const f of files) {
    if (list.length >= MAX_IMAGES) { problems.push(`At most ${MAX_IMAGES} images per message.`); break; }
    if (f.size > MAX_IMAGE_BYTES) { problems.push(`${f.name || "An image"} is over 10 MB.`); continue; }
    if (f.size === 0) { problems.push(`${f.name || "An image"} is empty.`); continue; }
    // Only `data:` previews: the page's CSP allows no blob: images.
    const preview = f.type.startsWith("image/") && f.size <= MAX_IMAGE_BYTES ? await readAsDataURL(f) : "";
    list.push({ name: `img-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`, file: f, preview, uploaded: false });
  }
  state.attachments.set(pane, list);
  if (problems.length) toast(problems.join(" ")); else render();
}

/** Uploads the images the hub does not hold yet. Progress over all of them. Null, or the failed reply. */
async function upload(list              )                                                      {
  const todo = list.filter((a) => !a.uploaded);
  const total = todo.reduce((n, a) => n + a.file.size, 0);
  let done = 0;
  for (const a of todo) {
    const bytes = new Uint8Array(await a.file.arrayBuffer());
    const parts = Math.max(1, Math.ceil(bytes.length / PART_BYTES));
    for (let part = 0; part < parts; part++) {
      const chunk = bytes.subarray(part * PART_BYTES, (part + 1) * PART_BYTES);
      const r = await request({ type: "attach", upload: a.name, part, parts, data: base64(chunk) });
      if (!r.ok) return { code: r.code, message: `${a.file.name || "An image"}: ${explain(r.code, r.message)}` };
      done += chunk.length;
      state.progress = total ? done / total : 1;
      render();
    }
    a.uploaded = true;
  }
  return null;
}

/** `pane`'s composer sent the draft it had at `rev`: the draft goes, wherever the page is now, unless it changed
 *  since (typed, or a pulled text placed): then it stays as it is. */
function sent(pane        , rev        )       {
  if ((state.revs[pane] ?? 0) !== rev) return;
  const field = document.getElementById("composer-text")                              ;
  if (field && state.composerPane === pane) { field.value = ""; fitText(field); }
  delete state.drafts[pane];
}

function composer(pane        , promptOpen         , agent         )                     {
  if (!canInput() || !find(pane)?.p.agent) return null;
  const box = h("textarea", { id: "composer-text", rows: 1, placeholder: promptOpen ? "Answer the prompt first" : agent ? `Reply to ${agent}` : "Message", disabled: promptOpen, "aria-label": "Message to the agent" });
  const images = state.attachments.get(pane) ?? [];
  const send = async ()                => {
    const rev = state.revs[pane] ?? 0;
    const text = box.value.trim();
    const list = state.attachments.get(pane) ?? [];
    if ((!text && !list.length) || state.busy) return;
    state.busy = true;
    state.progress = list.some((a) => !a.uploaded) ? 0 : null;
    render();
    const failed = list.length ? await upload(list) : null;
    const r = failed ? { ok: false, ...failed } : await request({ type: "send", pane, text, ...(list.length ? { attachments: list.map((a) => a.name) } : {}) });
    state.busy = false;
    state.progress = null;
    if (r.ok) {
      sent(pane, rev);
      state.attachments.delete(pane);
    } else {
      // The hub no longer knows an image (reconnected, or it dropped one): upload them all again next time.
      if (r.code === "badRequest") for (const a of list) a.uploaded = false;
      toast(failed ? failed.message ?? "The image did not go." : explain(r.code, r.message));
    }
    render();
  };
  box.addEventListener("input", () => { state.revs[pane] = (state.revs[pane] ?? 0) + 1; });
  box.addEventListener("keydown", (e               ) => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); send(); } });
  box.addEventListener("input", () => fitText(box));
  // Turning the phone re-wraps the text: fit it again when the field's width changes (its height changes are ours).
  let width = -1;
  new ResizeObserver(([e]) => {
    const w = e.contentRect.width;
    if (width >= 0 && w !== width) fitText(box);
    width = w;
  }).observe(box);
  // A file field with `image/*`: the phone offers the camera and the photo library.
  const file = h("input", { id: "composer-file", type: "file", accept: "image/*", multiple: true, hidden: true, tabindex: "-1",
    onchange: () => { const fs = [...(file.files ?? [])]; file.value = ""; pick(pane, fs); } });
  const attach = h("button", { id: "composer-attach", class: "attach", type: "button", disabled: promptOpen || state.busy || images.length >= MAX_IMAGES,
    "aria-label": "Add images", title: "Add images", onclick: () => file.click() }, "+");
  const thumbs = images.length ? h("div", { class: "thumbs", role: "list", "aria-label": "Images to send" }, ...images.map((a, i) =>
    h("div", { class: "thumb", role: "listitem" },
      a.preview ? h("img", { src: a.preview, alt: a.file.name || `Image ${i + 1}` }) : h("span", { class: "thumb-none" }, "?"),
      h("button", { class: "thumb-remove", type: "button", disabled: state.busy, "aria-label": `Remove ${a.file.name || `image ${i + 1}`}`, title: "Remove",
        onclick: () => { state.attachments.set(pane, images.filter((x) => x !== a)); render(); } }, "×")))) : null;
  const progress = state.progress !== null ? h("progress", { id: "composer-progress", max: 1, value: state.progress, "aria-label": "Sending images" }) : null;
  return h("form", { class: "composer glass-float", "data-pane": pane, onsubmit: (e       ) => { e.preventDefault(); send(); } },
    thumbs, progress,
    h("div", { class: "composer-row" }, file, attach, box,
      h("button", { id: "composer-send", class: "primary send", type: "submit", disabled: promptOpen || state.busy, "aria-label": "Send", title: "Send" }, "↑")));
}

/** The text field grows with its text up to its max-height, and back. From its current height to the new one, so the
 *  CSS transition runs: one line keeps the composer a pill, more lines open it into a rounded rectangle. */
function fitText(box                     )       {
  if (!box.isConnected) return;
  const from = box.getBoundingClientRect().height;
  box.style.height = "auto";
  const max = parseFloat(getComputedStyle(box).maxHeight) || Infinity;
  const to = Math.min(box.scrollHeight, max);
  box.style.overflowY = box.scrollHeight > max ? "auto" : "hidden";
  box.style.height = `${from}px`;
  void box.offsetHeight;   // the start height takes effect before the end one
  box.style.height = `${to}px`;
}

// --- Start.
if (pushReady()) worker();
render();
/** Asks `/hub.json` how this hub knows the owner: `pair` (the Mac hub) or `tailscale` (the Rust helper: no pairing). */
async function start()                {
  if (!state.token && !state.pairSecret) {
    try {
      const r = await fetch("/hub.json", { cache: "no-store" });
      const j = r.ok ? await r.json()                                       : null;
      if (j?.auth === "tailscale") { helper = { post: j.post ?? [] }; state.token = "tailscale"; render(); }
    } catch { /* no answer: pair as before */ }
  }
  if (state.token || state.pairSecret) connect();
}
void start();
follow();
