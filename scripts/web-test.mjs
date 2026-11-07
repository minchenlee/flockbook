// Headless browser test of the phone PWA against a fake hub (HerdrKitChecks --serve-web: TLS, 127.0.0.1, no herdr).
//   swift build && node scripts/web-test.mjs [--shots <dir>]      (CHROME_HEADLESS=<path> to pick the browser)
//   node scripts/web-test.mjs --live <session>   read-only smoke run: the app's own hub (HerdrDesk --serve-remote, headless,
//                                                 127.0.0.1, a read device) on a named herdr session. Sends no input.
// Drives Chrome's headless shell over the DevTools protocol with Node's built-in WebSocket: no packages, no window.
// The browser trusts only the hub's own certificate (SPKI pin), as a paired phone does.
import { createCDPSender, CDPCommandError } from "./lib/cdp-commands.mjs";
import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, existsSync, readdirSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir, homedir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { createInterface } from "node:readline";
import { inflateSync } from "node:zlib";

const onlyAt = process.argv.indexOf("--only");
const only = onlyAt > 0 ? process.argv[onlyAt + 1] : null;
if (onlyAt > 0 && !["transport", "earlier-held", "morph", "composer", "note"].includes(only)) throw new Error(`unknown web-test section: ${only}`);
const phase = (name) => console.log(`web-test: phase ${name}`);
// The phone page's morph and composer sections. Every wait is for a state (a morph's end, frames drawn), with a long
// limit: on a shared CI runner one morph took up to 3 s at CPU 4x, and fixed waits came out too short. The CPU is
// slowed 4x only with WEB_MORPH_STRICT=1 (a quiet machine, where frame times are checked too); else 2x
// (WEB_MORPH_CPU to change it).
const webStrict = process.env.WEB_MORPH_STRICT === "1";
const webCPU = webStrict ? 4 : Number(process.env.WEB_MORPH_CPU ?? 2);
const webLimit = 30000;
const webSettled = "window.__m.vt.every((v) => v.end > 0)";
const twoFrames = (p) => p.eval("new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r(true))))");
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const shotsAt = process.argv.indexOf("--shots");
const shots = shotsAt > 0 ? process.argv[shotsAt + 1] : null;
if (shots) mkdirSync(shots, { recursive: true });

function findChrome() {
  if (process.env.CHROME_HEADLESS) return process.env.CHROME_HEADLESS;
  const cache = join(homedir(), "Library/Caches/ms-playwright");
  const dirs = existsSync(cache) ? readdirSync(cache).filter((d) => d.startsWith("chromium_headless_shell-")).sort().reverse() : [];
  for (const d of dirs) {
    for (const sub of readdirSync(join(cache, d))) {
      const p = join(cache, d, sub, "chrome-headless-shell");
      if (existsSync(p)) return p;
    }
  }
  // A CI runner has Google Chrome (its own headless mode).
  const app = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
  if (existsSync(app)) return app;
  throw new Error("no chrome-headless-shell or Google Chrome found; pass its path");
}

let failures = 0, total = 0;
function check(ok, name) {
  total++;
  if (!ok) failures++;
  console.log(`${ok ? "ok" : "FAIL"} ${name}`);
}

// --- The fake hub.
const liveAt = process.argv.indexOf("--live");
const live = liveAt > 0 ? process.argv[liveAt + 1] : null;
// The app binary talks to the window server: never start it while the screen is locked.
if (live && /"CGSSessionScreenIsLocked"\s*=\s*Yes/.test(execFileSync("/usr/sbin/ioreg", ["-n", "Root", "-d1"]).toString())) {
  console.log("web-test: the screen is locked; the live run needs it unlocked. Nothing started.");
  process.exit(2);
}
// The herdr environment of a calling pane must not leak into the app (it would pick that session).
const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith("HERDR_")));
phase("hub startup");
const hub = live
  ? spawn(join(root, ".build/debug/HerdrDesk"), ["--session", live, "--serve-remote"], { stdio: ["pipe", "pipe", "ignore"], env })
  : spawn(join(root, ".build/debug/HerdrKitChecks"), ["--serve-web"], { stdio: ["pipe", "pipe", "inherit"],
      // Stills: the pictures in the conversation are the drawn ones (scripts/demo/pictures), not noise.
      env: shots ? { ...process.env, SERVE_WEB_PICTURES: join(root, "scripts/demo/pictures") } : process.env });
const info = await new Promise((resolve, reject) => {
  const rl = createInterface({ input: hub.stdout });
  rl.on("line", (l) => {
    if (l.startsWith("{")) resolve(JSON.parse(l));
    else if (l.includes("FAIL")) reject(new Error(l));
  });
  hub.once("exit", (c) => reject(new Error(`hub exited ${c}`)));
});
const spki = execFileSync("sh", ["-c", `/usr/bin/openssl x509 -in '${info.cert}' -pubkey -noout | /usr/bin/openssl pkey -pubin -outform der | /usr/bin/openssl dgst -sha256 -binary | base64`]).toString().trim();
const origin = `https://127.0.0.1:${info.port}`;

// --- The browser.
phase("headless browser startup");
const profile = mkdtempSync(join(tmpdir(), "fb-web-test-"));
const chrome = spawn(findChrome(), [
  "--headless", `--user-data-dir=${profile}`, "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
  `--ignore-certificate-errors-spki-list=${spki}`, "--window-size=390,844", "about:blank",
], { stdio: ["ignore", "ignore", "pipe"] });
const wsURL = await new Promise((resolve, reject) => {
  const rl = createInterface({ input: chrome.stderr });
  rl.on("line", (l) => { const m = l.match(/DevTools listening on (ws:\/\/\S+)/); if (m) resolve(m[1]); });
  chrome.once("exit", (c) => reject(new Error(`chrome exited ${c}`)));
});

const cdp = new WebSocket(wsURL);
await new Promise((r) => cdp.addEventListener("open", r, { once: true }));
const commands = createCDPSender(cdp), listeners = [];
cdp.addEventListener("message", (event) => {
  const message = JSON.parse(event.data);
  if (!message.id) for (const listener of listeners) listener(message);
});
function send(method, params = {}, sessionId) { return commands.send(method, params, sessionId); }

/** A fresh page (own storage when `context` is new). */
async function page(context, own = false) {
  // `own`: in a window of its own, so it is the window's shown page (Chrome's full headless mode keeps a page opened
  // beside another one in the background: hidden, no View Transitions).
  const { targetId } = await send("Target.createTarget", { url: "about:blank", browserContextId: context, ...(own ? { newWindow: true } : {}) });
  const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
  const errors = [];
  listeners.push((m) => {
    if (m.sessionId !== sessionId) return;
    if (m.method === "Runtime.exceptionThrown") errors.push(m.params.exceptionDetails.exception?.description ?? m.params.exceptionDetails.text);
    if (m.method === "Log.entryAdded" && m.params.entry.level === "error") errors.push(m.params.entry.text);
    if (m.method === "Runtime.consoleAPICalled" && m.params.type === "error") errors.push(m.params.args.map((a) => a.value ?? a.description).join(" "));
  });
  await send("Runtime.enable", {}, sessionId);
  await send("Log.enable", {}, sessionId);
  await send("Page.enable", {}, sessionId);
  await send("Emulation.setDeviceMetricsOverride", { width: 390, height: 844, deviceScaleFactor: 3, mobile: true }, sessionId);
  const p = {
    errors,
    async eval(expr) {
      const r = await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true }, sessionId);
      if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
      return r.result.value;
    },
    async goto(url) { await send("Page.navigate", { url }, sessionId); },
    /** A real reload (`goto` to the same address with another hash stays in the document). */
    async reload() { await send("Page.reload", {}, sessionId); },
    /** A DevTools call on this page. */
    async cdp(method, params = {}) { return send(method, params, sessionId); },
    /** Runs `source` in every page this target loads, before the page's own scripts. */
    async onEveryLoad(source) { await send("Page.addScriptToEvaluateOnNewDocument", { source }, sessionId); },
    /** Waits until `expr` is truthy; returns its value, or undefined after `ms`. */
    async until(expr, ms = 4000) {
      const end = Date.now() + ms;
      while (Date.now() < end) {
        try { const v = await p.eval(expr); if (v) return v; }
        catch (error) { if (error instanceof CDPCommandError && error.fatal) throw error; }
        await new Promise((r) => setTimeout(r, 50));
      }
      return undefined;
    },
    async close() { await send("Target.closeTarget", { targetId }); },
    async click(sel) { return p.eval(`(() => { const e = document.querySelector(${JSON.stringify(sel)}); if (!e || e.disabled) return false; e.click(); return true; })()`); },
    async type(sel, text) {
      return p.eval(`(() => { const e = document.querySelector(${JSON.stringify(sel)}); if (!e) return false; e.value = ${JSON.stringify(text)};
        e.dispatchEvent(new Event("input", { bubbles: true })); return true; })()`);
    },
    /** Puts files in a file field, as the phone's picker would. */
    async files(sel, paths) {
      const { root } = await send("DOM.getDocument", {}, sessionId);
      const { nodeId } = await send("DOM.querySelector", { nodeId: root.nodeId, selector: sel }, sessionId);
      await send("DOM.setFileInputFiles", { files: paths, nodeId }, sessionId);
    },
    /** Pixel ratio: the headless shell draws backdrop-filter (the blur) only at 1. */
    async scale(dpr, width = 390, height = 844) { await send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: dpr, mobile: true }, sessionId); },
    /** The page's colour scheme, as the phone's setting would switch it. */
    async scheme(dark) {
      await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: dark ? "dark" : "light" }] }, sessionId);
    },
    async shot(name, dark = false) {
      if (!shots) return;
      await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: dark ? "dark" : "light" }] }, sessionId);
      await new Promise((r) => setTimeout(r, 150));
      const { data } = await send("Page.captureScreenshot", { format: "png" }, sessionId);
      writeFileSync(join(shots, `${name}${dark ? "-dark" : ""}.png`), Buffer.from(data, "base64"));
    },
  };
  return p;
}
async function context() { return (await send("Target.createBrowserContext")).browserContextId; }

const text = "document.getElementById('app').innerText";
const noScroll = "document.documentElement.scrollWidth <= window.innerWidth";
const pages = [];

try {
  if (only === "transport") await transportRun();
  else if (only === "earlier-held") await heldEarlierRun();
  else if (only === "morph") await morph();
  else if (only === "composer") await composerPin();
  else if (only === "note") await agentNote();
  else if (live) await liveRun();
  else await fakeRun();
} catch (e) {
  check(false, `test run: ${e.stack ?? e}`);
} finally {
  for (const x of pages) await x.close().catch(() => {});
  cdp.close();
  const gone = new Promise((r) => chrome.once("exit", r));
  chrome.kill();
  await gone;
  hub.stdin.end();
  await new Promise((r) => hub.once("exit", r));
  // Chrome's helpers may still write to the profile after the browser exits (ENOTEMPTY on a CI runner): a few tries,
  // then a warning. The checks alone decide the result.
  try { rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 }); }
  catch (e) { console.log(`web-test: warning: the browser profile was not removed (${e.code ?? e})`); }
}
console.log(failures ? `web-test: ${failures} of ${total} FAILED` : `web-test: OK ${total} checks passed`);
process.exit(failures ? 1 : 0);

/** A short real-browser check of command matching, navigation and concurrent evaluations. */
async function transportRun() {
  phase("CDP transport");
  const p = await page(await context());
  pages.push(p);
  await p.goto(origin + "/");
  check(await p.until("document.title === 'Flockbook'"), "transport: navigation completes");
  const values = await Promise.all([p.eval("1 + 1"), p.eval("new Promise(r => setTimeout(() => r(7), 10))"), p.eval("3 * 3")]);
  check(JSON.stringify(values) === "[2,7,9]", "transport: concurrent replies match their commands");
}

/** Hold the real earlier request at the socket boundary; its arrival resolves the observer directly. */
async function installEarlierParking(p) {
  const delay = Math.max(0, Number(process.env.WEB_TEST_PARK_DELAY_MS) || 0);
  await p.eval(`(() => { const send = WebSocket.prototype.send;
    WebSocket.prototype.send = function (data) {
      if (window.__parkEarlier && typeof data === 'string' && data.includes('"loadEarlier"')) {
        window.__parkEarlier = 0;
        const ready = () => { window.__parked = () => send.call(this, data); window.__onParked?.(); };
        if (${delay} > 0) setTimeout(ready, ${delay}); else ready();
        return;
      }
      return send.call(this, data);
    }; })()`);
}
async function parkEarlierAtTop(p, readRow) {
  // A scroll event requires a changed offset. A previous held case can leave the page already at zero.
  await p.eval("window.__parkEarlier = 0; window.scrollTo(0, document.documentElement.scrollHeight)");
  if (!await p.until("window.scrollY > 0")) throw new Error("earlier: could not position the reader above the top gesture");
  const at = await p.eval(`new Promise(resolve => {
    window.__parkEarlier = 1; window.__parked = null;
    window.__onParked = () => { window.__onParked = null; resolve(${readRow}); };
    window.dispatchEvent(new WheelEvent('wheel', { deltaY: -400 })); window.scrollTo(0, 0);
  })`);
  // Settle layout frames, rather than guessing how long a wheel's event takes on a loaded host.
  await p.eval("new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))");
  return JSON.parse(at);
}
async function heldEarlierRun() {
  phase("earlier held gesture");
  const p = await page(await context()); pages.push(p);
  await p.goto(origin + "/");
  await p.eval("localStorage.setItem('flockbook.token', 'input-token')");
  await p.goto(origin + "/#pane=w2%3Ap2");
  if (!await p.until("!!document.querySelector('#composer-text')", 8000)) throw new Error("earlier: input page did not load");
  hub.stdin.write("session w2:p2 held-check 60\nrich held-check 600\n");
  if (!await p.until("document.body.innerText.includes('New session reply 59')", 8000)) throw new Error("earlier: held fixture did not load");
  await installEarlierParking(p);
  const rows = "document.querySelectorAll('.items > .msg').length";
  const read = `(() => { const e = [...document.querySelectorAll('.items > .msg')].find(e => e.getBoundingClientRect().bottom > 0);
    return JSON.stringify(e ? { text: e.textContent, top: e.getBoundingClientRect().top } : { text: '', top: NaN }); })()`;
  const at = await parkEarlierAtTop(p, read), count = await p.eval(rows);
  await p.cdp("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x: 195, y: 420 }] });
  await p.eval("window.__parked()");
  if (!await p.until("document.getElementById('load-earlier')?.disabled === false", 8000)) throw new Error("earlier: held request did not finish");
  const top = `[...document.querySelectorAll('.items > .msg')].find(e => e.textContent === ${JSON.stringify(at.text)})?.getBoundingClientRect().top`;
  check(await p.eval(rows) === count && Math.abs(await p.eval(top) - at.top) <= 2, "earlier-held: completed reply waits while a real touch is held");
  await p.cdp("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  check(await p.until(`${rows} > ${count}`, 8000) === true, "earlier-held: release inserts the waiting rows");
}

/** The real app hub, a read device: list, one conversation, no input controls. */
async function liveRun() {
  phase("live read-only");
  const p = await page(await context());
  pages.push(p);
  await p.goto(origin + "/");
  await p.eval(`localStorage.setItem('flockbook.token', ${JSON.stringify(info.token)})`);
  await p.goto(origin + "/");
  check(await p.until("!!document.querySelector('.list')", 8000) === true, "live: agent list after hello");
  const panes = await p.eval("[...document.querySelectorAll('[data-pane]')].map(e => e.dataset.pane)");
  check(panes.length > 0, `live: ${panes.length} agents listed`);
  check(await p.eval("document.querySelector('.level').textContent") === "Read only", "live: read device");
  check(await p.eval(noScroll), "live: no sideways scroll");
  await p.shot("live-list");
  if (panes.length) {
    await p.click(`[data-pane='${panes[0]}']`);
    check(await p.until("document.querySelectorAll('.msg, .logged, .tools, .screen').length > 0", 8000) === true, `live: ${panes[0]} shows its conversation or screen`);
    check(await p.eval("!document.querySelector('#allow') && !document.querySelector('.composer')") === true, "live: no input controls for a read device");
    check(await p.eval(noScroll), "live: conversation, no sideways scroll");
    await p.shot("live-conversation");
  }
  check(p.errors.length === 0, `live: no errors or CSP violations (${p.errors.join(" | ")})`);
}

/** The fake hub: every screen and every action. */
async function fakeRun() {
  await morph();   // first: its frame times, with no other page open in the browser
  await composerPin();
  await agentNote();
  phase("pairing and page assets");
  // The page and the socket on one origin, scripts under the CSP.
  const fresh = await page(await context());
  pages.push(fresh);
  await fresh.goto(origin + "/");
  check(await fresh.until("!!document.querySelector('#pair-words')") === true, "pair: without a token the page asks for the six words");
  check(await fresh.eval("document.title") === "Flockbook", "page: title");
  check(await fresh.eval("fetch('/manifest.webmanifest').then(r => r.json()).then(m => m.name)") === "Flockbook", "page: manifest");
  // Icons (the Mac app icon, scripts/pwa-icons.swift): each one in the manifest and the page's links is served as a
  // PNG of its stated size; the maskable and touch icons are opaque; the maskable artwork stays in the 80% circle.
  const icons = JSON.parse(await fresh.eval(`(async () => {
    const m = await (await fetch('/manifest.webmanifest')).json();
    const links = [...document.querySelectorAll('link[rel~=icon], link[rel=apple-touch-icon]')]
      .map((l) => ({ src: l.getAttribute('href'), sizes: l.getAttribute('sizes') || '180x180', purpose: l.rel }));
    const out = [];
    for (const i of [...m.icons.map((i) => ({ ...i, purpose: 'manifest ' + (i.purpose || 'any') })), ...links]) {
      const r = await fetch('/' + i.src);
      const blob = await r.blob();
      const bmp = await createImageBitmap(blob);
      const c = new OffscreenCanvas(bmp.width, bmp.height), g = c.getContext('2d');
      g.drawImage(bmp, 0, 0);
      const d = g.getImageData(0, 0, bmp.width, bmp.height).data;
      let opaque = true, reach = 0;
      for (let y = 0; y < bmp.height; y++) for (let x = 0; x < bmp.width; x++) {
        const o = (y * bmp.width + x) * 4;
        if (d[o + 3] < 255) opaque = false;
        if (d[o] < 150 && d[o + 3] > 200) reach = Math.max(reach, Math.hypot(x + 0.5 - bmp.width / 2, y + 0.5 - bmp.height / 2) / bmp.width);
      }
      out.push({ src: i.src, sizes: i.sizes, purpose: i.purpose, status: r.status, type: r.headers.get('content-type'), w: bmp.width, h: bmp.height, opaque, reach });
    }
    return JSON.stringify(out);
  })()`));
  check(icons.length >= 6 && icons.every((i) => i.status === 200 && i.type === "image/png" && i.sizes === `${i.w}x${i.h}`),
    `icons: every manifest and page icon is a PNG of its stated size (${icons.map((i) => `${i.src} ${i.type} ${i.w}x${i.h}`).join(", ")})`);
  check(["192x192", "512x512"].every((z) => icons.some((i) => i.purpose === "manifest any" && i.sizes === z)), "icons: manifest has 192 and 512 (any)");
  const mask = icons.find((i) => i.purpose === "manifest maskable"), touch = icons.find((i) => i.purpose === "apple-touch-icon");
  check(mask?.w === 512 && mask.opaque && mask.reach > 0.2 && mask.reach <= 0.4, `icons: maskable 512 is opaque, artwork inside the 80% circle (reach ${mask?.reach.toFixed(3)} of the side)`);
  check(touch?.w === 180 && touch.opaque, "icons: apple-touch-icon is 180 and opaque");
  check(icons.filter((i) => i.purpose === "icon").length === 2, "icons: favicon for light and dark");
  await fresh.shot("pair");
  await fresh.shot("pair", true);

  // Wrong words: an error, no token.
  await fresh.type("#pair-words", "wrong words here and there now");
  await fresh.click("#pair-go");
  check(await fresh.until(`${text}.includes('wrong or expired')`) === true, "pair: wrong words -> error shown");
  check(await fresh.eval("localStorage.getItem('flockbook.token')") === null, "pair: no token kept after a failure");

  // The QR link: the secret is read from the fragment and removed from the address bar; pairing gives a read device.
  await fresh.goto(`${origin}/#pair=${info.pairSecret}&fp=${info.fingerprint}`);
  check(await fresh.until("!!document.querySelector('#pair-go') && !document.querySelector('#pair-words')") === true, "pair: QR link -> no words needed");
  check(await fresh.eval("location.hash") === "", "pair: secret removed from the address bar");
  check((await fresh.eval(text)).includes(info.fingerprint.slice(0, 16)), "pair: certificate fingerprint shown for reference");
  await fresh.type("#pair-name", "Test phone");
  await fresh.click("#pair-go");
  check(await fresh.until("!!document.querySelector('.list')") === true, "pair: paired -> agent list");
  check((await fresh.eval("localStorage.getItem('flockbook.token')"))?.length > 20, "pair: device token kept");
  check(await fresh.eval("document.querySelector('.level').textContent") === "Read only", "read: level shown as Read only");

  // A read device: cards say so, no buttons, no composer.
  await fresh.click("[data-pane='w1:p1']");
  check(await fresh.until("!!document.querySelector('.card.approval')") === true, "read: approval card shown");
  check(await fresh.eval("!document.querySelector('#allow') && !document.querySelector('#deny') && !document.querySelector('.composer')") === true,
    "read: no Allow/Deny, no composer");
  check((await fresh.eval(text)).includes("Read only"), "read: card says read only");

  phase("notifications");
  // Notifications: the hub has a push key, so the list offers them; the worker is /sw.js on this origin. The default
  // browser context: Chrome has no Push API in the other (incognito) contexts and logs an error there.
  const np = await page();
  pages.push(np);
  await np.goto(origin + "/");
  await np.eval("localStorage.setItem('flockbook.token', 'input-token')");
  await np.goto(origin + "/");
  check(await np.until("document.querySelector('#notify')?.textContent === 'Notify me'") === true, "push: the list offers notifications");
  const sw = await np.until("navigator.serviceWorker.getRegistration().then(r => r && (r.active ?? r.waiting ?? r.installing)?.scriptURL)", 6000);
  check(sw === origin + "/sw.js", `push: service worker registered (${sw})`);
  await np.click("#notify");
  check(await np.until("document.querySelector('.toast')?.textContent.includes('blocked')") === true && await np.eval("document.querySelector('#notify').textContent") === "Notify me",
    "push: permission not given -> says so, stays off");
  await send("Browser.grantPermissions", { permissions: ["notifications"], origin });
  await np.click("#notify");
  // The headless shell has no push service: turning on fails there and says so; a browser with one turns on.
  const pushed = await np.until("(t => t.includes('Notifications on') || t.includes('did not turn on'))(document.querySelector('.toast')?.textContent ?? '') && document.querySelector('.toast').textContent", 8000);
  check(!!pushed && !await np.eval("document.querySelector('#notify').disabled"), `push: permission given -> tries to subscribe, answers (${pushed})`);
  // The page's half with a stand-in push service: the browser's subscription goes to the hub, which keeps it (an FCM
  // endpoint, the RFC 8291 example keys); a second tap turns it off again.
  // Kept across reloads (localStorage), as a browser keeps its subscription.
  await np.onEveryLoad(`(() => {
    const ep = 'https://fcm.googleapis.com/fcm/send/web-test';
    const make = (key) => ({ endpoint: ep, options: { applicationServerKey: key },
      toJSON: () => ({ endpoint: ep, keys: { p256dh: 'BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4', auth: 'BTBZMqHH6r4Tts7J_aSIgg' } }),
      unsubscribe: async () => { localStorage.removeItem('fake-sub'); return true; } });
    const saved = () => { const k = localStorage.getItem('fake-sub'); return k ? make(Uint8Array.from(JSON.parse(k)).buffer) : null; };
    PushManager.prototype.getSubscription = async () => saved();
    PushManager.prototype.subscribe = async (o) => { localStorage.setItem('fake-sub', JSON.stringify([...o.applicationServerKey])); return saved(); };
    window.__sub = () => localStorage.getItem('fake-sub');
  })()`);
  await np.goto(origin + "/");
  check(await np.until("document.querySelector('#notify')?.textContent === 'Notify me'") === true, "push: reloaded, still off");
  await np.eval("document.querySelector('.toast')?.remove()");
  await np.click("#notify");
  check(await np.until("document.querySelector('#notify')?.textContent === 'Notifications on'", 6000) === true
    && (await np.eval("document.querySelector('.toast')?.textContent ?? ''")).startsWith("Notifications on"), "push: subscribed -> the hub keeps it, the button says on");
  await np.shot("list-notify");
  // `__old` marks the page before a reload, so a check never passes on the old page.
  await np.eval("window.__old = true");
  await np.goto(origin + "/");
  check(await np.until("!window.__old && document.querySelector('#notify')?.textContent === 'Notifications on'", 6000) === true, "push: reloaded -> the hub still has it (welcome pushOn), on");
  await np.click("#notify");
  check(await np.until("document.querySelector('#notify')?.textContent === 'Notify me' && window.__sub() === null", 6000) === true, "push: tapped again -> off, unsubscribed");
  // The hub no longer has it (as after Remove on the Mac) but the browser still does: the page drops its own.
  const hubKey = [...Buffer.from("BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8", "base64url")];
  await np.eval(`localStorage.setItem('fake-sub', ${JSON.stringify(JSON.stringify(hubKey))})`);
  await np.eval("window.__old = true");
  await np.goto(origin + "/");
  check(await np.until("!window.__old && !!document.querySelector('#notify') && window.__sub() === null", 6000) === true
    && await np.eval("document.querySelector('#notify').textContent") === "Notify me", "push: the hub does not have it (removed on the Mac) -> the page drops its own, off");

  // An unknown token: back to pairing with a reason.
  const gone = await page(await context());
  pages.push(gone);
  await gone.goto(origin + "/");
  await gone.eval("localStorage.setItem('flockbook.token', 'no-such-token')");
  await gone.goto(origin + "/");
  check(await gone.until(`${text}.includes('not paired')`) === true, "token: unknown token -> pair screen with a reason");
  check(await gone.eval("localStorage.getItem('flockbook.token')") === null, "token: unknown token removed");

  phase("input device");
  // An input device.
  const p = await page(await context());
  pages.push(p);
  await p.goto(origin + "/");
  await p.eval("localStorage.setItem('flockbook.token', 'input-token')");
  await p.goto(origin + "/");
  check(await p.until("!!document.querySelector('.list')") === true, "list: shown after hello");
  const order = await p.eval("[...document.querySelectorAll('[data-pane]')].map(e => e.dataset.pane)");
  check(JSON.stringify(order.slice(0, 2)) === JSON.stringify(["w1:p1", "w1:p2"]) && order.length === 4, `list: Needs you first (${order})`);
  check(await p.eval("document.querySelector('#needs-you').textContent") === "Needs you · 2", "list: Needs you count");
  check(await p.eval("document.title") === "(2) Flockbook", "list: count in the title");
  check(await p.eval("document.querySelectorAll('.dot.blocked').length") === 2 && await p.eval("document.querySelectorAll('.dot.working').length") === 1,
    "list: status shapes");
  check(await p.eval(noScroll), "list: no sideways scroll at 390 pt");
  // No bar that stays on top: the title and the input level are plain content and scroll away with the list.
  const head = "document.querySelector('.list-head')";
  check(await p.eval(`!!${head} && !document.querySelector('.list .top') && !['sticky', 'fixed'].includes(getComputedStyle(${head}).position)`) === true,
    "list: the title is not a sticky bar");
  check(await p.eval(`${head}.querySelector('h1').textContent === 'Agents' && ['Input on', 'Read only'].includes(${head}.querySelector('.level').textContent)`) === true,
    "list: title and input level still shown");
  await p.shot("list");
  await p.shot("list", true);
  // Scrolled (page-only filler rows, gone on the next render): the title has gone up with the content.
  await p.eval(`(() => { const g = document.querySelector('.list .group:last-of-type');
    for (let i = 0; i < 12; i++) { const li = document.createElement('li'); li.innerHTML = '<div class="row glass-float"><span class="row-text"><span class="row-title">Filler agent ' + (i + 1) + '</span><span class="row-sub">claude · filler</span></span></div>'; g.append(li); }
    window.scrollTo(0, 260); })()`);
  check(await p.eval(`window.scrollY > 0 && ${head}.getBoundingClientRect().bottom <= 0`) === true, "list: scrolled, the title is out of view");
  await p.shot("list-scrolled");
  await p.shot("list-scrolled", true);
  await p.eval("window.scrollTo(0, 0)");

  phase("approval and markdown");
  // Conversation with an approval.
  await p.click("[data-pane='w1:p1']");
  check(await p.until("!!document.querySelector('#allow')") === true, "approval: card with Allow");
  check(await p.eval("getComputedStyle(document.querySelector('.top')).position") === "sticky", "conversation: the back bar stays on top");
  check(await p.eval("location.hash") === "#pane=w1%3Ap1", "nav: pane in the address (back button works)");
  check(await p.eval("document.querySelector('.msg.assistant strong')?.textContent") === "src/app.ts", "conversation: markdown bold");
  check(await p.eval("document.querySelector('.msg.assistant pre code')?.textContent") === "npm run build", "conversation: code block");
  check(await p.eval("document.querySelector('.chip summary')?.textContent") === "Ran 2 commands", "conversation: tool chip");
  // Full agent markdown, as the Mac reading view shows it: rendered into a test message below the app, in the same
  // column, so a render of the conversation (an older page loading by itself) does not take it away.
  const md = [
    "## Plan", "", "Some *italic*, **bold** and ~~old~~ text.", "",
    "- one", "  - nested", "- two", "", "1. first", "2. second", "",
    "| Step | Result |", "|---|:---:|", "| build | `ok` |", "| test | **2 failed** |", "",
    "1. Run it:", "   ```sh", "   npm test", "   ```", "2. Read the output.", "",
    "> quoted *line*", "", "---", "",
    "| " + Array.from({ length: 8 }, (_, i) => `column_${i}_with_a_long_unbroken_name`).join(" | ") + " |",
    "|" + "---|".repeat(8), "| " + Array.from({ length: 8 }, (_, i) => `value_${i}_that_does_not_wrap_at_all`).join(" | ") + " |", "",
    "<img src=x onerror=\"window.__xss=1\"> [bad](javascript:window.__xss=2) [ok](https://example.com/a)", "",
    "| # | 專案 | 是什麼 |", "|---|---|---|",
    "| 1 | `~/Library/Caches/ExampleApp/paste/` 裡 1 個 1-byte 的 png | 測試留下的暫存檔 |",
    "| 2 | 視窗位置 | 誤開視窗那 8 次留下的視窗位置 |",
  ].join("\n");
  await p.eval(`(async () => { const { markdown } = await import('/markdown.js');
    const d = document.createElement('div'); d.className = 'msg assistant'; d.id = 'md-test'; d.append(markdown(${JSON.stringify(md)}));
    const host = document.createElement('main'); host.id = 'md-host'; const items = document.createElement('div'); items.className = 'items';
    items.append(d); host.append(items); document.body.append(host); })()`);
  const q = (s) => `document.querySelector('#md-test ${s}')`;
  check(await p.eval(`${q("h2")}?.textContent`) === "Plan", "markdown: ## heading");
  check(await p.eval(`${q("em")}?.textContent`) === "italic" && await p.eval(`${q("del")}?.textContent`) === "old", "markdown: *italic* and ~~strike~~");
  check(await p.eval(`${q("ul > li > ul > li")}?.textContent`) === "nested" && await p.eval(`document.querySelectorAll('#md-test > ul > li').length`) === 2,
    "markdown: - list with a nested item");
  check(await p.eval(`document.querySelectorAll('#md-test > ol')[0]?.children.length`) === 2, "markdown: 1. ordered list");
  check(await p.eval(`${q("table thead th")}?.textContent`) === "Step" && await p.eval(`${q("table tbody tr:last-child td:last-child strong")}?.textContent`) === "2 failed"
    && await p.eval(`getComputedStyle(${q("table tbody td:last-child")}).textAlign`) === "center", "markdown: GFM table (header, inline, alignment)");
  check(await p.eval(`document.querySelectorAll('#md-test > ol')[1]?.querySelector('li pre code')?.textContent`) === "npm test",
    "markdown: fence indented inside a list item");
  check(await p.eval(`${q("blockquote em")}?.textContent`) === "line" && await p.eval(`!!${q("hr")}`) === true, "markdown: > quote and --- rule");
  // Outside rendered tables (a "#" header cell is fine), no line still starts with # or |.
  check(await p.eval(`(() => { const c = document.querySelector('#md-test').cloneNode(true); c.querySelectorAll('.table-wrap').forEach(t => t.remove());
    document.body.append(c); const t = c.innerText; c.remove(); return !/^#|^\\|/m.test(t); })()`) === true, "markdown: no raw # or | lines left");
  check(await p.eval(`(() => { const w = document.querySelectorAll('#md-test .table-wrap')[1]; return !!w && w.scrollWidth > w.clientWidth; })()`) === true
    && await p.eval(noScroll), "markdown: a wide table scrolls in its own box, the page does not");
  check(await p.eval(`!${q("img")} && !window.__xss && [...document.querySelectorAll('#md-test a')].map(a => a.href).join() === 'https://example.com/a'`) === true,
    "markdown: HTML stays text, only http(s) links");
  // CJK and long inline code at a phone column: every cell keeps a readable width (no one-character-per-line
  // column), no row grows tall, and the table box scrolls sideways instead.
  const cjk = await p.eval(`(() => { const t = document.querySelectorAll('#md-test .table-wrap')[2]; if (!t) return null;
    return { col: document.querySelector('#md-test').clientWidth, cell: Math.min(...[...t.querySelectorAll('td:not(.short), th:not(.short)')].map(c => c.getBoundingClientRect().width)),
      row: Math.max(...[...t.querySelectorAll('tr')].map(r => r.getBoundingClientRect().height)),
      short: t.querySelector('th').getBoundingClientRect().width }; })()`);
  check(!!cjk && cjk.cell >= 96 && cjk.row <= 110 && cjk.short < 60 && await p.eval(noScroll),
    `markdown: CJK table keeps readable columns (column ${cjk?.col} px, narrowest cell ${Math.round(cjk?.cell)} px, tallest row ${Math.round(cjk?.row)} px, # column ${Math.round(cjk?.short)} px)`);
  // Hostile nesting renders (capped) instead of exhausting the stack.
  check(await p.eval(`(async () => { const { markdown } = await import('/markdown.js');
    const f = markdown('>'.repeat(20000) + ' deep\\n\\n' + '- '.repeat(20000) + 'deep');
    return f.textContent.includes('deep'); })()`) === true, "markdown: deep > and - nesting is capped");
  // Hostile input: every case renders, in bounded time and nodes (a throw would drop the whole message).
  const hostile = {
    "table 500 columns x 500 rows": "'|'.repeat(501) + '\\n' + '|-'.repeat(500) + '|\\n' + Array(500).fill('|').join('\\n')",
    "table 130k rows": "'a|b\\n-|-\\n' + Array(130000).fill('|').join('\\n')",
    "inline nesting": "'__a'.repeat(10000) + '__'.repeat(10000)",
    "__ run": "'__'.repeat(9990) + 'x'",
    "** run": "'**'.repeat(9990) + 'x'",
    "[ run": "'['.repeat(19990)",
    "delimiter row spaces": "'a|b\\n' + ' '.repeat(30000) + 'x'",
    "fence spaces": "'```' + ' '.repeat(30000) + '`'",
    "heading spaces": "'# a' + ' '.repeat(30000) + 'x'",
    "empty - item": "'-'",
    "empty 1. item": "'1.'",
    "heading closing # run": "'# ' + '#'.repeat(40000) + 'x'",
    "~ fence with U+2028": "'~'.repeat(40000) + '\\u2028'",
    "` fence with U+2028": "'`'.repeat(19990) + '\\u2028'",
    "item spaces with U+2028": "'- ' + ' '.repeat(40000) + 'x\\u2028'",
    "130k empty items": "'-\\n'.repeat(130000)",
    "200k paragraphs": "'a\\n\\n'.repeat(200000)",
    "budget edge: rules then *x*": "'---\\n'.repeat(19999) + '\\n*x*'",
    "budget edge: rules then nested": "'---\\n'.repeat(19995) + '\\n# ' + '**a *b ~~c **d**~~*'.repeat(3)",
    "table cells full of *a*": "['a|b', '-|-', ...Array(4999).fill('*a* '.repeat(100) + '|' + '*a* '.repeat(100))].join('\\n')",
  };
  for (const [name, src] of Object.entries(hostile)) {
    const r = await p.eval(`(async () => { const { markdown } = await import('/markdown.js');
      const s = ${src}, t = performance.now();
      try { const f = markdown(s); const d = document.createElement('div'); d.append(f);
        return { ms: performance.now() - t, nodes: d.querySelectorAll('*').length };
      } catch (e) { return { error: String(e) }; } })()`);
    check(!r.error && r.ms < 250 && r.nodes <= 20000,
      `markdown hostile: ${name} (${r.error ?? `${Math.round(r.ms)} ms, ${r.nodes} nodes`})`);
  }
  check(await p.eval(`(async () => { const { markdown } = await import('/markdown.js');
    const d = document.createElement('div'); d.append(markdown('-\\n- b\\n\\n1.'));
    return d.querySelectorAll('ul > li').length === 2 && d.querySelectorAll('ol > li').length === 1; })()`) === true,
    "markdown: an empty - or 1. item is a list item");
  // A workspace's monogram, as the app's (ChecksMonogram.swift): an apostrophe inside a word does not split it.
  const monoCases = [["Brandon's Brain", "BB"], ["Brandon’s Brain", "BB"], ["O'Neil Lab", "OL"], ["O’Neil Lab", "OL"], ["Don't Panic", "DP"],
    ["[ME] Meadow · Migration", "MM"], ["OpenAtlas", "OA"], ["flockbook", "FL"], ["[ME]", "ME"]];
  const monoGot = JSON.parse(await p.eval(`(async () => { const { monogram } = await import('/monogram.js');
    return JSON.stringify(${JSON.stringify(monoCases.map(([n]) => n))}.map(monogram)); })()`));
  for (const [i, [name, want]] of monoCases.entries()) check(monoGot[i] === want, `monogram: ${name} -> ${want} (${monoGot[i]})`);
  await p.eval("document.querySelector('#md-test').scrollIntoView()");
  await p.shot("markdown");
  await p.shot("markdown", true);
  await p.eval("document.querySelectorAll('#md-test .table-wrap')[2].scrollIntoView({ block: 'center' })");
  await p.shot("markdown-end");
  await p.shot("markdown-end", true);
  await p.eval("document.querySelector('#md-host').remove()");
  check(await p.eval("document.querySelector('.card.approval .command')?.textContent") === "npm run build", "approval: command shown");
  check(await p.eval("document.querySelector('#composer-text')?.disabled") === true, "composer: off while a prompt is open");
  check(await p.eval(noScroll), "conversation: no sideways scroll");
  await p.shot("approval");
  await p.shot("approval", true);
  await p.click("#load-earlier");
  check(await p.until("document.querySelectorAll('.msg.assistant').length === 4") === true, "conversation: Load earlier prepends older items");
  check(await p.eval("document.querySelector('.msg.assistant').textContent") === "Older message 1.", "conversation: older items first");
  await p.click("#allow");
  check(await p.until(`${text}.includes('Build passed.')`) === true, "approval: Allow -> agent goes on");
  check(await p.until("!document.querySelector('.card.approval')") === true, "approval: card gone");
  check(await p.until("document.title === '(1) Flockbook'") === true, "approval: one less needs you");

  phase("questions");
  // A question.
  await p.eval("location.hash = ''");
  await p.until("!!document.querySelector('.list')");
  await p.click("[data-pane='w1:p2']");
  check(await p.until("document.querySelectorAll('.option').length === 2") === true, "question: two options");
  check((await p.eval("document.querySelector('.option').textContent")).includes("Recommended"), "question: recommended marked");
  check(await p.eval("document.querySelector('#submit-answer').disabled") === true, "question: no answer, no send");
  await p.shot("question");
  await p.shot("question", true);
  await p.click(".option");
  check(await p.eval("document.querySelector('.option').getAttribute('aria-pressed')") === "true", "question: option picked");
  await p.click("#submit-answer");
  check(await p.until(`${text}.includes('Deploying to the chosen target.')`) === true, "question: answer sent, agent goes on");

  phase("composer");
  // Composer on an idle agent, and the in-app alert when another agent starts to wait.
  await p.eval("location.hash = '#pane=w2%3Ap2'");
  check(await p.until("document.querySelector('#composer-text')?.disabled === false") === true, "composer: on for an idle agent");
  await p.type("#composer-text", "hello from the phone");
  await p.click("#composer-send");
  check(await p.until("[...document.querySelectorAll('.msg.user')].some(e => e.textContent === 'hello from the phone')") === true, "composer: text sent and shown");
  check(await p.eval("document.querySelector('#composer-text').value") === "", "composer: cleared after send");

  const screenOpen = "document.querySelector('details.screen')?.open";
  // Untouched, the Screen keeps following its default: open while the pane has no rows, closed once rows come. (Built
  // open, the page gets a toggle event of its own; that is not the reader's choice.) A session with no rows, here.
  hub.stdin.write("session w2:p2 s30 0\n");
  check(await p.until(`${screenOpen} === true && !document.querySelector('.items .msg')`) === true, "screen default: open while the pane has no rows");
  await new Promise((r) => setTimeout(r, 1500));   // several hub ticks, each re-renders the page
  check(await p.eval(screenOpen) === true, "screen default: still open after ticks");
  await p.type("#composer-text", "the first row");
  await p.click("#composer-send");
  check(await p.until("[...document.querySelectorAll('.msg.user')].some(e => e.textContent === 'the first row')") === true, "screen default: a row comes");
  check(await p.until(`${screenOpen} === false`) === true, `screen default: closed once rows come, the reader never touched it (${await p.eval(screenOpen)})`);
  hub.stdin.write("session w2:p2 s4\n");
  check(await p.until("[...document.querySelectorAll('.msg')].some(e => e.textContent.includes('Notes are up to date.'))") === true, "screen default: back to the notes session");

  // Compacting: the screen changes on every hub tick. The text field stays the same element (a phone keyboard closes
  // when its field leaves the page), with its text, caret and focus; a message sent meanwhile goes to the agent.
  await p.type("#composer-text", "compact me");
  await p.click("#composer-send");
  check(await p.until("(document.querySelector('.screen pre')?.textContent ?? '').includes('Compacting conversation')") === true, "compacting: the agent compacts");
  await p.eval("(() => { const e = document.querySelector('#composer-text'); window.__box = e; e.focus(); e.value = 'half a thought'; e.setSelectionRange(4, 4); return true; })()");
  const compactScreen = await p.eval("document.querySelector('.screen pre').textContent");
  check(await p.until(`document.querySelector('.screen pre')?.textContent !== ${JSON.stringify(compactScreen)}`) === true, "compacting: the screen moves on");
  await new Promise((r) => setTimeout(r, 700));
  check(await p.eval("document.querySelector('#composer-text') === window.__box") === true, "compacting: screen updates keep the same text field");
  check(await p.eval("(() => { const e = window.__box; return document.activeElement === e && e.value === 'half a thought' && e.selectionStart === 4 && !e.disabled; })()") === true,
    "compacting: the text, caret and focus stay; the field stays on");
  await p.type("#composer-text", "sent while compacting");
  await p.click("#composer-send");
  check(await p.until("[...document.querySelectorAll('.msg.user')].some(e => e.textContent === 'sent while compacting')") === true, "compacting: a message sent meanwhile goes to the agent");
  await p.type("#composer-text", "Typing while it compacts");
  await p.shot("composer-compacting");
  await p.shot("composer-compacting", true);
  // Screen: opened by the reader, it stays open while hub ticks bring a new screen (each re-renders the page); closed
  // again, it stays closed.
  const twoTicks = async (what) => {
    for (const n of [1, 2]) {
      const before = await p.eval("document.querySelector('.screen pre')?.textContent ?? ''");
      check(await p.until(`(document.querySelector('.screen pre')?.textContent ?? '') !== ${JSON.stringify(before)}`) === true, `${what}: tick ${n} brings a new screen`);
    }
  };
  check(await p.eval(screenOpen) === false && await p.click("details.screen summary") === true && await p.eval(screenOpen) === true, "screen: the reader opens it");
  await twoTicks("screen open");
  check(await p.eval(screenOpen) === true, "screen: still open after two ticks");
  await p.click("details.screen summary");
  await twoTicks("screen closed");
  check(await p.eval(screenOpen) === false, "screen: closed again, stays closed after two ticks");
  await p.type("#composer-text", "compact done");
  await p.click("#composer-send");
  check(await p.until("!(document.querySelector('.screen pre')?.textContent ?? '').includes('Compacting')") === true, "compacting: done");

  // The composer's shape: one line is a pill (radius = half the height); more lines open it into a rounded rectangle
  // with the same radius, the text field's height easing between the two.
  const form = "document.querySelector('form.composer')", field = "document.querySelector('#composer-text')";
  const shape = `(() => { const f = ${form}, r = f.getBoundingClientRect(); return { h: r.height, radius: parseFloat(getComputedStyle(f).borderTopLeftRadius), box: ${field}.getBoundingClientRect().height }; })()`;
  const setText = (t) => p.eval(`(() => { const e = ${field}; e.value = ${JSON.stringify(t)}; e.dispatchEvent(new Event('input')); return e.getAnimations().length; })()`);
  await setText("One line");
  await new Promise((r) => setTimeout(r, 400));
  const one = await p.eval(shape);
  check(Math.abs(one.radius - one.h / 2) < 1, `pill: one line, radius ${one.radius} = half of ${one.h}`);
  await p.shot("composer-pill");
  await p.shot("composer-pill", true);
  const easing = await setText("A longer note for the agent.\nSecond line.\nThird line.\nFourth line.");
  check(easing > 0, "pill: the height eases (a running transition), no jump");
  await new Promise((r) => setTimeout(r, 400));
  const four = await p.eval(shape);
  check(four.box > one.box + 60 && Math.abs(four.radius - one.radius) < 0.5 && four.h > four.radius * 2 + 40,
    `pill: four lines, a rounded rectangle (height ${four.h}, radius ${four.radius})`);
  await p.shot("composer-multiline");
  await p.shot("composer-multiline", true);
  await setText("");
  await new Promise((r) => setTimeout(r, 400));
  const back = await p.eval(shape);
  check(Math.abs(back.h - one.h) < 1, `pill: emptied, back to one line (${back.h})`);
  // Landscape to portrait: a long draft wraps into more lines; the field grows with no typing, nothing is cut off.
  const metrics = (width, height) => p.scale(3, width, height);
  const draft = "A long draft typed with the phone turned sideways, words enough to fill one wide line and wrap once the phone stands up again.";
  await metrics(844, 390);
  await setText(draft);
  await new Promise((r) => setTimeout(r, 400));
  const wide = await p.eval(shape);
  await metrics(390, 844);
  await new Promise((r) => setTimeout(r, 500));
  const fits = `(() => { const e = ${field}; return { fit: e.scrollHeight <= e.clientHeight + 1 || getComputedStyle(e).overflowY === 'auto', box: e.getBoundingClientRect().height }; })()`;
  const narrow = await p.eval(fits);
  check(narrow.fit && narrow.box > wide.box, `turn: portrait re-wraps the draft and the field grows (${wide.box} -> ${narrow.box}), nothing cut off`);
  await setText("");

  phase("image attachments");
  // Images from the phone: pick, preview, remove, send in parts with progress; the hub re-encodes them.
  const imgDir = mkdtempSync(join(tmpdir(), "fb-web-img-"));
  const big = join(imgDir, "big.png"); writeFileSync(big, Buffer.alloc(11 * 1024 * 1024, 1));
  const fake = join(imgDir, "fake.png"); writeFileSync(fake, "this is not an image");
  const icon192 = join(root, "web/icon-192.png"), icon32 = join(root, "web/favicon-32.png"), icon512 = join(root, "web/icon-512.png");
  const thumbs = "document.querySelectorAll('.thumb').length";
  check(await p.eval("(() => { const f = document.querySelector('#composer-file'); return f.type === 'file' && f.accept === 'image/*' && f.multiple; })()") === true,
    "image: the attach field takes images, several at once (camera or photos)");
  await p.files("#composer-file", [icon192, icon32]);
  check(await p.until(`${thumbs} === 2`) === true, "image: two picked -> two thumbnails");
  check(await p.eval("[...document.querySelectorAll('.thumb img')].every(i => i.src.startsWith('data:image/') && i.complete && i.naturalWidth > 0)") === true,
    "image: thumbnails are data: previews (CSP), drawn");
  await p.shot("attach");
  await p.shot("attach", true);
  await p.click(".thumb-remove");
  check(await p.until(`${thumbs} === 1`) === true, "image: remove takes one off");
  await p.files("#composer-file", [big]);
  check(await p.until("document.querySelector('.toast')?.textContent.includes('over 10 MB')") === true && await p.eval(thumbs) === 1,
    "image: over 10 MB refused on the phone, with a message");
  await p.files("#composer-file", [icon512, icon512, icon512, icon512]);
  check(await p.until(`${thumbs} === 4`) === true && await p.until("document.querySelector('.toast')?.textContent.includes('At most 4')") === true,
    "image: at most 4 per message, with a message");
  check(await p.eval("document.querySelector('#composer-attach').disabled") === true, "image: attach button off at 4");
  // Watch the progress bar while it sends.
  await p.eval("window.__progress = false; new MutationObserver(() => { if (document.querySelector('#composer-progress')) window.__progress = true; }).observe(document.body, { subtree: true, childList: true })");
  await p.type("#composer-text", "look at these");
  await p.click("#composer-send");
  check(await p.until("[...document.querySelectorAll('.msg.user')].some(e => e.textContent === 'look at these [4 images 32x32 512x512 512x512 512x512]')", 8000) === true,
    "image: text + 4 images sent, re-encoded at their size");
  check(await p.eval("window.__progress") === true, "image: a progress bar while uploading");
  check(await p.until(`${thumbs} === 0 && !document.querySelector('#composer-progress')`) === true && await p.eval("document.querySelector('#composer-text').value") === "",
    "image: composer empty after send");
  await p.files("#composer-file", [icon192]);
  await p.until(`${thumbs} === 1`);
  await p.click("#composer-send");
  check(await p.until("[...document.querySelectorAll('.msg.user')].some(e => e.textContent === '[1 image 192x192]')", 8000) === true, "image: an image with no text sends");
  await p.files("#composer-file", [fake]);
  await p.until(`${thumbs} === 1`);
  await p.type("#composer-text", "fake one");
  await p.click("#composer-send");
  check(await p.until("document.querySelector('.toast')?.textContent.includes('fake.png: Not a PNG, JPEG, HEIC or WebP image.')") === true,
    "image: the hub refuses a non-image (magic bytes, not the name); the message says which");
  check(await p.eval(`${thumbs} === 1 && document.querySelector('#composer-text').value === 'fake one'`) === true, "image: after a refusal the text and image stay");
  check(![...await p.eval("[...document.querySelectorAll('.msg.user')].map(e => e.textContent)")].includes("fake one"), "image: nothing sent after a refusal");
  await p.click(".thumb-remove");
  await p.type("#composer-text", "");
  rmSync(imgDir, { recursive: true, force: true });
  await p.shot("composer");
  await p.shot("composer", true);
  // Frosted glass like the app's materials: the bar and the composer blur what scrolls under them, over a see-through fill.
  const glass = (sel) => `(() => { const c = getComputedStyle(document.querySelector(${JSON.stringify(sel)}));
    const f = c.backdropFilter || c.webkitBackdropFilter || ""; const a = /rgba\\([^)]*,\\s*([0-9.]+)\\)/.exec(c.backgroundColor);
    return f.includes("blur") && !!a && Number(a[1]) < 1; })()`;
  check(await p.eval(glass(".top > .back")) === true && await p.eval(glass(".top > .bar-title")) === true && await p.eval(glass(".composer")) === true,
    "look: the back button, the name capsule and the composer are frosted glass");
  // Stills with text under the glass (page-only filler, gone on the next render), at pixel ratio 1: see `scale`.
  if (shots) {
    await p.scale(1);
    await p.eval(`(() => { const items = document.querySelector(".items");
      for (let i = 0; i < 14; i++) { const d = document.createElement("div"); d.className = "msg assistant"; d.textContent = "Filler line " + (i + 1) + " scrolls under the frosted bar and the composer."; items.prepend(d); }
      window.scrollTo(0, 240); })()`);
    await p.shot("glass");
    await p.shot("glass", true);
    await p.scale(3);
  }
  await p.type("#composer-text", "block me");
  await p.click("#composer-send");
  check(await p.until("!!document.querySelector('.banner')") === true, "alert: banner when another agent needs you");
  await p.shot("banner");
  await p.shot("banner", true);
  await p.click(".banner");
  check(await p.until("location.hash === '#pane=w2%3Ap1'") === true, "alert: banner opens that agent");

  // A Claude question read from the screen only: shown, answered on the Mac (no answer controls).
  await p.eval("location.hash = '#pane=w2%3Ap2'");
  await p.until("document.querySelector('#composer-text')?.disabled === false");
  // Messages Claude took mid-turn (queued_command in its log) show as the owner's, in log order.
  await p.type("#composer-text", "queued log");
  await p.click("#composer-send");
  const users = "[...document.querySelectorAll('.msg.user')].map(e => e.textContent)";
  check(await p.until(`${users}.includes('Thanks.')`) === true, "queued: the log's messages arrive");
  const queuedOrder = await p.eval(`(() => { const u = ${users}; return [u.indexOf('Run the tests, then summarize.'), u.indexOf('Also check the lint step.'), u.indexOf('Here is the failing screen.'), u.indexOf('Thanks.')]; })()`);
  check(queuedOrder[0] >= 0 && queuedOrder[0] < queuedOrder[1] && queuedOrder[1] < queuedOrder[2] && queuedOrder[2] < queuedOrder[3], `queued: mid-turn messages shown as user text, in log order (${queuedOrder})`);
  check(await p.eval(`!${users}.some(t => t.includes('task-notification') || t.includes('Subagent') || t.includes('Sidechain'))`) === true,
    "queued: task notices, subagent hand-backs and sidechains stay hidden");
  await p.eval("window.scrollTo(0, document.body.scrollHeight)");
  await p.shot("queued");
  await p.shot("queued", true);
  phase("queued input");
  // The agent's own queue: faded "Queued" bubbles at the end, and Pull back puts it in the composer.
  await p.type("#composer-text", "queue me");
  await p.click("#composer-send");
  const queued = "[...document.querySelectorAll('.msg.user.queued')].map(e => e.textContent)";
  check(await p.until(`${queued}.length === 2`) === true, "queue: two queued bubbles");
  check(await p.eval(`${queued}.every(t => t.startsWith('Queued'))`) === true && await p.eval("document.querySelector('.items').lastElementChild.classList.contains('queue')") === true,
    "queue: labelled Queued, at the end of the conversation");
  check((await p.eval("document.querySelector('.queue-foot').textContent")).includes("All 2 come back as one message."), "queue: the note says Claude merges them");
  await p.until("!document.querySelector('.toast')", 8000);   // an earlier test's notice would cover the button
  await p.eval("window.scrollTo(0, document.body.scrollHeight)");
  await p.shot("queue");
  await p.shot("queue", true);
  await p.type("#composer-text", "My draft");
  await p.click("#pull-queued");
  check(await p.until("document.querySelector('#composer-text').value === 'My draft\\nAlso run the lint step before you commit.\\nThen open a draft PR with a short summary.'") === true,
    "queue: Pull back puts the text after the draft, on its own line");
  check(await p.until("!document.querySelector('.queue')") === true, "queue: the bubbles go when the agent's queue is empty");
  await p.shot("queue-pulled");
  await p.shot("queue-pulled", true);
  phase("status line");
  // Claude Code's status line (the Mac's switch on): a row of tags over the composer's field (model, effort, context
  // with its bar, cost marked as an estimate, then the owner's line cut into parts without the repeats), the chevron at
  // the right; open: every tag, wrapped, in the parts' colours, and when it was updated; no raw line (appendix D).
  await p.type("#composer-text", "status me");
  await p.click("#composer-send");
  const sline = "document.querySelector('form.composer > .statusline')";
  const rowTags = `[...document.querySelectorAll('form.composer > .statusline > .status-head > .status-tags > .status-tag')]`;
  const shownTags = `${rowTags}.filter((t) => t.offsetTop === t.parentElement.firstElementChild.offsetTop).map((t) => t.textContent)`;
  const more = "document.querySelector('form.composer > .statusline > .status-head > .status-more')";
  check(await p.until(`${shownTags}.slice(0, 4).join('|') === 'Opus 5.5|high|ctx 42%|$3.27 est.' && ${shownTags}.length < ${rowTags}.length`) === true
    && await p.eval(`${rowTags}.map((t) => t.textContent).join('|')`) === "Opus 5.5|high|ctx 42%|$3.27 est.|⎇ main|turn $0.18|day $11.40/$25|month $212/$300|rate limit 5h: 23%",
    `status line: Claude Code's tags, then the owner's parts without the repeats; one row (${await p.eval(`JSON.stringify(${shownTags})`)})`);
  check(await p.eval(`${sline} === document.querySelector('form.composer').firstElementChild
    && ${sline}.getBoundingClientRect().bottom <= document.querySelector('form.composer > .composer-row').getBoundingClientRect().top
    && ${sline}.getBoundingClientRect().height < 34 && !document.querySelector('.status-detail') && ${noScroll}`) === true,
    `status line: one row over the composer's field (${await p.eval(`${sline}?.getBoundingClientRect().height`)} px); the page does not scroll sideways`);
  check(await p.eval(`+getComputedStyle(${rowTags}[0]).fontWeight >= 600 && ${rowTags}[2].querySelector('.status-fill')?.style.width === '42%'`) === true,
    "status line: the model in bold; the context's bar at 42%");
  check(await p.eval(`!!${more} && !${more}.hidden && ${more}.getAttribute('aria-label') === 'Show the whole status line' && ${more}.getAttribute('aria-expanded') === 'false'`) === true,
    "status line: tags left out: the chevron at the right, with a label");
  check(await p.eval(`!!${more}.querySelector('.chev.up')`) === true, "status line: closed, the chevron points up (the tags open upwards)");
  await p.eval("window.scrollTo(0, document.body.scrollHeight)");
  await p.shot("statusline");
  await p.shot("statusline", true);
  await p.click("form.composer .status-more");
  const openTags = "[...document.querySelectorAll('.statusline.open > .status-head > .status-tags.all > .status-tag')]";
  check(await p.until(`${openTags}.length === 9`) === true && await p.eval(`!document.querySelector('.status-lines, .status-row') && ${noScroll}`) === true,
    "status line: open, every tag wrapped; no raw line; the page does not scroll sideways");
  const dark = await p.eval("matchMedia('(prefers-color-scheme: dark)').matches");
  const part = (t) => `${openTags}.find((e) => e.textContent === ${JSON.stringify(t)})`;
  const red = await p.eval(`getComputedStyle(${part("month $212/$300")}.children[1]).color`), cyan = await p.eval(`getComputedStyle(${part("⎇ main")}.firstElementChild).color`);
  check(dark ? cyan === "rgb(0, 205, 205)" && red !== "rgb(205, 0, 0)" : red === "rgb(205, 0, 0)" && cyan !== "rgb(0, 205, 205)",
    `status line: a part's colour where it reads on the tag, else the tag's text colour (${dark ? "dark" : "light"}: red ${red}, cyan ${cyan})`);
  const dimEl = `${part("rate limit 5h: 23%")}.firstElementChild`;
  const dimSeen = await p.eval(`[getComputedStyle(${dimEl}).color, getComputedStyle(${dimEl}).opacity].join(' ')`);
  check(dimSeen === (dark ? "rgb(150, 150, 150) 1" : "rgb(107, 107, 107) 1"), `status line: dim text is a fixed grey that reads on the tag (${dimSeen})`);
  // The phone switches light or dark with the line kept (not rebuilt): the parts' colours follow.
  await p.scheme(!dark);
  await p.until("false", 200);
  const red2 = await p.eval(`getComputedStyle(${part("month $212/$300")}.children[1]).color`), cyan2 = await p.eval(`getComputedStyle(${part("⎇ main")}.firstElementChild).color`);
  check(!dark ? cyan2 === "rgb(0, 205, 205)" && red2 !== "rgb(205, 0, 0)" : red2 === "rgb(205, 0, 0)" && cyan2 !== "rgb(0, 205, 205)",
    `status line: switched to ${dark ? "light" : "dark"}, the parts' colours follow (red ${red2}, cyan ${cyan2})`);
  await p.scheme(dark);
  check(await p.eval(`${openTags}.map((t) => t.textContent).join('|')`) === "Opus 5.5|high|ctx 42%|$3.27 est.|⎇ main|turn $0.18|day $11.40/$25|month $212/$300|rate limit 5h: 23%"
    && /^Updated \d/.test(await p.eval("document.querySelector('.status-detail > .status-time')?.textContent ?? ''")),
    `status line: open, every tag and when it was updated (${await p.eval("document.querySelector('.status-time')?.textContent")})`);
  check(await p.eval(`${more}.getAttribute('aria-expanded') === 'true' && ${more}.getAttribute('aria-label') === 'Hide the whole status line'`) === true,
    "status line: open, the chevron says it hides");
  check(await p.eval(`!!${more}.querySelector('.chev') && !${more}.querySelector('.chev.up')`) === true, "status line: open, the chevron points down");
  await p.eval("window.scrollTo(0, document.body.scrollHeight)");
  await p.shot("statusline-open");
  await p.shot("statusline-open", true);
  // Open is per pane: another Claude pane's line starts closed; back here, this one is still open.
  await p.eval("location.hash = '#pane=w1%3Ap1'");
  await p.until("location.hash === '#pane=w1%3Ap1' && !!document.querySelector('#composer-text')");
  await p.type("#composer-text", "status me");
  await p.click("#composer-send");
  check(await p.until("!!document.querySelector('.statusline') && !document.querySelector('.statusline.open')") === true,
    "status line: open is per pane: another pane's line starts closed");
  await p.type("#composer-text", "status off");
  await p.click("#composer-send");
  await p.until("!document.querySelector('.statusline')");
  await p.eval("location.hash = '#pane=w2%3Ap2'");
  check(await p.until("location.hash === '#pane=w2%3Ap2' && !!document.querySelector('.statusline.open')") === true,
    "status line: back to the first pane, its line is still open");
  await p.eval("window.__keptLine = document.querySelector('.statusline')");
  await p.until("false", 1500);
  check(await p.eval("window.__keptLine === document.querySelector('.statusline')") === true, "status line: hub news does not rebuild an unchanged line");
  await p.click("form.composer .status-more");
  check(await p.until("!document.querySelector('.status-detail')") === true, "status line: a second tap closes it");
  // A narrow phone: the tags that fit, then the chevron.
  await p.scale(3, 300, 844);
  check(await p.until(`${shownTags}.length > 0 && ${shownTags}.length < 4 && !${more}.hidden && ${noScroll}`) === true,
    `status line: narrow, the tags that fit and the chevron (${await p.eval(`JSON.stringify(${shownTags})`)})`);
  await p.scale(3);
  // A nearly full context: the context tag and its bar orange (the orange that reads on the tag, light or dark).
  await p.type("#composer-text", "status hot");
  await p.click("#composer-send");
  check(await p.until(`${rowTags}[2]?.classList.contains('warn') && ${rowTags}[2].textContent === 'ctx 86%'`) === true
    && await p.eval(`getComputedStyle(${rowTags}[2]).color`) === (await p.eval("matchMedia('(prefers-color-scheme: dark)').matches") ? "rgb(255, 160, 74)" : "rgb(160, 75, 0)"),
    `status line: context 86%, orange (${await p.eval(`getComputedStyle(${rowTags}[2]).color`)})`);
  await p.eval("window.scrollTo(0, document.body.scrollHeight)");
  await p.shot("statusline-hot");
  await p.shot("statusline-hot", true);
  // Facts, no own line: every tag fits, nothing more to open, no chevron.
  await p.type("#composer-text", "status tags");
  await p.click("#composer-send");
  check(await p.until(`${shownTags}.length === 4 && ${more}?.hidden === true`) === true, "status line: facts only, all fit: no chevron");
  // No facts (Claude Code's JSON not read): the owner's parts are the tags.
  await p.type("#composer-text", "status plain");
  await p.click("#composer-send");
  check(await p.until(`${rowTags}.map((t) => t.textContent).slice(0, 3).join('|') === 'Opus 5.5|⎇ main|ctx 42%' && !document.querySelector('.status-plain')`) === true,
    `status line: no facts: the owner's parts as tags (${await p.eval(`JSON.stringify(${rowTags}.map((t) => t.textContent))`)})`);
  await p.type("#composer-text", "status off");
  await p.click("#composer-send");
  check(await p.until("!document.querySelector('.statusline')") === true, "status line: none from the hub: nothing shown");
  await p.type("#composer-text", "queue busy");
  await p.click("#composer-send");
  check(await p.until("document.querySelector('#pull-queued')?.disabled === true") === true, "queue: text in the agent's input box: Pull back is off");
  check((await p.eval("document.querySelector('.queue-foot').textContent")).includes("input box has text"), "queue: and says why");
  await p.type("#composer-text", "queue stop");
  await p.click("#composer-send");
  await p.until("document.querySelector('#pull-queued')?.disabled === false");
  await p.type("#composer-text", "");
  await p.click("#pull-queued");
  check(await p.until("(document.querySelector('.toast')?.textContent ?? '').includes('turn stopped')") === true, "queue: a stopped turn shows a notice");
  check(await p.eval("document.querySelector('#composer-text').value.startsWith('Also run the lint step')") === true, "queue: the text still comes back");
  // A pull that stopped before the agent's box was empty: the text, and a notice to check the terminal.
  await p.type("#composer-text", "");
  await p.type("#composer-text", "queue partial");
  await p.click("#composer-send");
  await p.until("!document.querySelector('.toast')", 8000);
  await p.until("document.querySelector('#pull-queued')?.disabled === false");
  await p.type("#composer-text", "");
  await p.click("#pull-queued");
  check(await p.until("(document.querySelector('.toast')?.textContent ?? '').includes('Check the agent') && document.querySelector('#composer-text').value.startsWith('Also run the lint step')") === true,
    "queue: a pull that stopped early: the text, and a notice that says why");
  // A pull QueuePull refuses before any key (too long to check exactly): a notice, the queue stays, nothing comes back.
  await p.type("#composer-text", "");
  await p.type("#composer-text", "queue long");
  await p.click("#composer-send");
  await p.until("!document.querySelector('.toast')", 8000);
  await p.until("document.querySelector('#pull-queued')?.disabled === false");
  await p.type("#composer-text", "");
  await p.click("#pull-queued");
  check(await p.until("(document.querySelector('.toast')?.textContent ?? '').includes('too long')") === true && await p.eval("document.querySelector('#composer-text').value") === "",
    "queue: a refused pull: the notice says why, nothing in the composer");
  check(await p.eval("document.querySelectorAll('.msg.user.queued').length") === 1, "queue: a refused pull: the queue is still shown");
  await p.eval("window.scrollTo(0, document.body.scrollHeight)");
  await p.shot("queue-refused");
  await p.shot("queue-refused", true);
  await p.until("!document.querySelector('.toast')", 8000);
  await p.eval("window.scrollTo(0, document.body.scrollHeight)");
  check(await p.until("document.querySelectorAll('.msg.user.queued').length === 1 && !!document.querySelector('#pull-queued')") === true,
    "queue: after the notice: the queue and Pull back as they were");
  await p.shot("queue-refused-after");
  // Codex: its queue is shown, with no Pull back; the note says to edit it in Codex's terminal.
  await p.type("#composer-text", "codex queue");
  await p.click("#composer-send");
  await p.until("!document.querySelector('.toast')", 8000);
  await p.eval("location.hash = '#pane=w2%3Ap1'");
  check(await p.until("document.querySelectorAll('.msg.user.queued').length === 1") === true, "queue: Codex's queued message is shown");
  check(await p.eval("!document.querySelector('#pull-queued') && document.querySelector('.queue-foot').textContent.includes('Shift+Left')") === true,
    "queue: Codex: no Pull back, the note says Shift+Left in its terminal");
  await p.eval("window.scrollTo(0, document.body.scrollHeight)");
  await p.shot("queue-codex");
  await p.shot("queue-codex", true);
  // Grok Build: the same, the note says Ctrl+; in Grok's terminal.
  await p.type("#composer-text", "grok queue");
  await p.click("#composer-send");
  await p.until("!document.querySelector('.toast')", 8000);
  await p.eval("location.hash = '#pane=w2%3Ap3'");
  check(await p.until("document.querySelectorAll('.msg.user.queued').length === 1") === true, "queue: Grok's queued message is shown");
  check(await p.eval("!document.querySelector('#pull-queued') && document.querySelector('.queue-foot').textContent.includes(\"Ctrl+; in Grok's terminal\")") === true,
    "queue: Grok: no Pull back, the note says Ctrl+; in its terminal");
  await p.eval("window.scrollTo(0, document.body.scrollHeight)");
  await p.shot("queue-grok");
  await p.type("#composer-text", "grok gone");
  await p.click("#composer-send");
  await p.until("!document.querySelector('.toast')", 8000);
  await p.eval("location.hash = '#pane=w2%3Ap2'");
  await p.until("location.hash === '#pane=w2%3Ap2' && !!document.querySelector('#composer-text')");
  await p.type("#composer-text", "");
  // A pull answered while another agent is open: the text waits for its own agent's composer.
  await p.type("#composer-text", "");
  await p.type("#composer-text", "queue slow");
  await p.click("#composer-send");
  await p.until("!document.querySelector('.toast')", 8000);   // the last notice would cover the button
  // The hub has taken "queue slow" (the queue stayed on screen after the refused pull, so the button is on already).
  await p.until("[...document.querySelectorAll('.msg.user')].some(e => e.textContent === 'queue slow')");
  await p.until("document.querySelector('#pull-queued')?.disabled === false");
  await p.type("#composer-text", "");
  await p.click("#pull-queued");
  await p.eval("location.hash = '#pane=w1%3Ap2'");
  await p.until("!!document.querySelector('#composer-text') && location.hash === '#pane=w1%3Ap2'");
  await p.until("false", 2500);
  check(await p.eval("document.querySelector('#composer-text').value") === "", "queue: a pull's text never goes into another agent's composer");
  await p.eval("location.hash = '#pane=w2%3Ap2'");
  check(await p.until("document.querySelector('#composer-text')?.value === 'Slow one.'") === true,
    `queue: back on its agent, its composer has it (${JSON.stringify(await p.eval("[location.hash, document.querySelector('#composer-text')?.value, document.querySelector('#composer-text')?.disabled]"))})`);
  // A slow pull answered after the reader scrolled up: the text goes into the composer, the page stays where they read.
  // The sticky composer is always in view here, so the headless page would not move anyway; a phone may scroll to a
  // focused field, so every focus of the composer must also pass preventScroll.
  await p.eval(`(() => { window.__focus = []; const f = HTMLElement.prototype.focus; window.__realFocus = f;
    HTMLElement.prototype.focus = function (o) { if (this.id === 'composer-text') window.__focus.push(o?.preventScroll === true); return f.call(this, o); }; })()`);
  await p.type("#composer-text", "");
  await p.type("#composer-text", "queue slow");
  await p.click("#composer-send");
  await p.until("!document.querySelector('.toast')", 8000);
  await p.until("[...document.querySelectorAll('.msg.user')].filter(e => e.textContent === 'queue slow').length === 2");
  await p.until("document.querySelector('#pull-queued')?.disabled === false");
  await p.type("#composer-text", "");
  await p.click("#pull-queued");
  await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: -100 })); window.scrollTo(0, 0)");
  const tall = await p.eval("document.documentElement.scrollHeight > window.innerHeight + 200");
  await p.until("document.querySelector('#composer-text')?.value === 'Slow one.'");
  await p.until("false", 300);
  check(tall === true && await p.eval("document.querySelector('#composer-text')?.value") === "Slow one." && await p.eval("Math.round(window.scrollY)") === 0,
    `queue: a slow pull's text arrives while scrolled up -> the page does not move (tall ${tall}, y ${await p.eval("Math.round(window.scrollY)")})`);
  const focuses = await p.eval("JSON.stringify(window.__focus)");
  check(await p.eval("window.__focus.length > 0 && window.__focus.every(Boolean)") === true, `queue: the pulled text focuses the composer without scrolling (preventScroll per focus: ${focuses})`);
  await p.eval("HTMLElement.prototype.focus = window.__realFocus");
  await p.eval("window.scrollTo(0, document.documentElement.scrollHeight)");
  await p.type("#composer-text", "");
  // Texts the hub kept (the phone was gone before they arrived) come after hello, each into its own agent's composer,
  // added up; once there, the hub forgets them (a reload brings nothing back).
  await p.type("#composer-text", "keep three");
  await p.click("#composer-send");
  await p.until("false", 600);
  await p.eval("window.__old = true; location.reload()");
  check(await p.until("!window.__old && document.querySelector('#composer-text')?.value === 'Kept one.\\nKept two.'", 8000) === true,
    `queue: after a reload the kept texts for this agent, both, in order (${JSON.stringify(await p.eval("document.querySelector('#composer-text')?.value"))})`);
  await p.eval("location.hash = '#pane=w1%3Ap2'");
  check(await p.until("location.hash === '#pane=w1%3Ap2' && document.querySelector('#composer-text')?.value === 'Kept for the other agent.'") === true,
    `queue: and the other agent's in its own composer (${JSON.stringify(await p.eval("[location.hash, document.querySelector('#composer-text')?.value, document.querySelector('#composer-text')?.disabled]"))})`);
  await p.eval("window.__old = true; location.reload()");
  await p.until("!window.__old && !!document.querySelector('#composer-text')", 8000);
  await p.until("false", 1000);
  check(await p.eval("document.querySelector('#composer-text').value") === "", "queue: confirmed texts do not come back after another reload");
  await p.eval("location.hash = '#pane=w2%3Ap2'");
  await p.until("location.hash === '#pane=w2%3Ap2' && !!document.querySelector('#composer-text')");
  await p.until("false", 500);
  check(await p.eval("document.querySelector('#composer-text').value") === "", "queue: nor for the first agent");
  await p.type("#composer-text", "");
  phase("send identity");
  // r2 P1-1: a send answered while another agent is open clears the composer it was sent from, never the open one.
  await p.type("#composer-text", "slow send");
  await p.click("#composer-send");
  await p.until("document.querySelector('#composer-text')?.value === '' && document.querySelector('#composer-send')?.disabled === false");
  await p.until("false", 400);
  await p.type("#composer-text", "Message A");
  await p.click("#composer-send");
  await p.eval("location.hash = '#pane=w1%3Ap2'");
  await p.until("location.hash === '#pane=w1%3Ap2' && !!document.querySelector('#composer-text')");
  await p.type("#composer-text", "B draft");
  await p.until("false", 2500);
  check(await p.eval("document.querySelector('#composer-text').value") === "B draft",
    `send: r2 P1-1: a send that ends while another agent is open leaves that agent's composer alone (${JSON.stringify(await p.eval("document.querySelector('#composer-text').value"))})`);
  await p.eval("location.hash = '#pane=w2%3Ap2'");
  await p.until("location.hash === '#pane=w2%3Ap2' && !!document.querySelector('#composer-text')");
  await p.until("false", 300);
  check(await p.eval("document.querySelector('#composer-text').value") === "",
    `send: r2 P1-1: the sent text is gone from its own agent's composer (${JSON.stringify(await p.eval("document.querySelector('#composer-text').value"))})`);
  await p.eval("location.hash = '#pane=w1%3Ap2'");
  await p.until("location.hash === '#pane=w1%3Ap2' && !!document.querySelector('#composer-text')");
  await p.type("#composer-text", "");
  await p.eval("location.hash = '#pane=w2%3Ap2'");
  await p.until("location.hash === '#pane=w2%3Ap2' && !!document.querySelector('#composer-text')");
  await p.type("#composer-text", "");
  // r3 P2-2: the composer changed while its send was out: the reply leaves the new text alone (also the same text
  // typed again).
  for (const [first, next] of [["fix", "fix\nnew instructions"], ["same", "same"]]) {
    await p.type("#composer-text", "slow send");
    await p.click("#composer-send");
    await p.until("document.querySelector('#composer-text')?.value === '' && document.querySelector('#composer-send')?.disabled === false");
    await p.until("false", 400);
    await p.type("#composer-text", first);
    await p.click("#composer-send");
    await p.until("false", 200);
    await p.type("#composer-text", next);
    await p.until("document.querySelector('#composer-send')?.disabled === false", 5000);
    await p.until("false", 300);
    check(await p.eval("document.querySelector('#composer-text').value") === next,
      `send: r3 P2-2: text written while the send was out stays (${JSON.stringify([first, next, await p.eval("document.querySelector('#composer-text').value")])})`);
    await p.type("#composer-text", "");
  }
  // r3 P2-3: a kept text with a warning, sent again while another agent is open: the warning comes with the text
  // when it goes into its own composer.
  await p.type("#composer-text", "keep warned");
  await p.click("#composer-send");
  await p.until("false", 600);
  await p.eval("window.__old = true; location.reload()");
  await p.until("!window.__old && !!document.querySelector('#composer-text')", 8000);
  await p.until("!document.querySelector('.toast')", 8000);
  await p.eval("location.hash = '#pane=w1%3Ap2'");
  check(await p.until("location.hash === '#pane=w1%3Ap2' && document.querySelector('#composer-text')?.value === 'Warned one.' && (document.querySelector('.toast')?.textContent ?? '').includes('Check the agent')") === true,
    `queue: r3 P2-3: placed later, the text comes with its warning (${JSON.stringify(await p.eval("[document.querySelector('#composer-text')?.value, document.querySelector('.toast')?.textContent]"))})`);
  await p.type("#composer-text", "");
  await p.eval("location.hash = '#pane=w2%3Ap2'");
  await p.until("location.hash === '#pane=w2%3Ap2' && !!document.querySelector('#composer-text')");
  await p.until("!document.querySelector('.toast')", 8000);
  // Older rows: a long session. Near the top they load by themselves, one page at a time, and the row that was on top
  // stays where it was on screen. "Load earlier" still loads (keyboard, screen reader, a retry). A tick does not
  // rebuild rows that did not change.
  const rowCount = "document.querySelectorAll('.items > .msg').length";
  await p.type("#composer-text", "long history");
  await p.click("#composer-send");
  check(await p.until("!!document.querySelector('#load-earlier')") === true, "earlier: a long session offers older rows");
  await p.eval("window.__row = document.querySelector('.items > .msg')");
  await p.type("#composer-text", "one more");
  await p.click("#composer-send");
  await p.until(`${users}.includes('one more')`);
  check(await p.eval("!!window.__row?.isConnected && document.querySelector('.items > .msg') === window.__row") === true,
    "earlier: a new row leaves the other rows' nodes in place (no full re-render)");
  await new Promise((r) => setTimeout(r, 500));
  const r0 = await p.eval(rowCount);
  // A wheel first: the reader's own scroll (a script's scroll leaves the page following the newest rows).
  const top0 = JSON.parse(await p.eval(`(() => { window.dispatchEvent(new WheelEvent('wheel', { deltaY: -400 })); window.scrollTo(0, 0);
    const e = [...document.querySelectorAll('.items > .msg')].find((e) => e.getBoundingClientRect().bottom > 0);
    return JSON.stringify(e ? { text: e.textContent, top: e.getBoundingClientRect().top } : { text: "", top: NaN }); })()`));
  check(await p.until(`${rowCount} >= ${r0 + 100}`, 4000) === true, `earlier: at the top, older rows load without a press (${r0} -> ${await p.eval(rowCount)})`);
  await new Promise((r) => setTimeout(r, 800));
  const top1 = await p.eval(`[...document.querySelectorAll('.items > .msg')].find((e) => e.textContent === ${JSON.stringify(top0.text)})?.getBoundingClientRect().top ?? NaN`);
  check(await p.eval(rowCount) > r0 && Math.abs(top1 - top0.top) < 2,
    `earlier: after the load, the row that was on top stays in place (${Math.round(top0.top)} -> ${Math.round(top1)})`);
  check(await p.eval(rowCount) === r0 + 100, `earlier: one page per visit to the top, not all at once (${await p.eval(rowCount)})`);
  await p.shot("earlier-autoload");
  await p.shot("earlier-autoload", true);
  const r1 = await p.eval(rowCount);
  await p.click("#load-earlier");
  check(await p.until(`${rowCount} >= ${r1 + 100}`, 4000) === true, `earlier: Load earlier still loads (${r1} -> ${await p.eval(rowCount)})`);
  // A very long session: auto-loading stops near 1000 rows. A soft cap: the check is before a load, so the last page
  // may take it past 1000 (to 1099 at most). The button still loads more.
  await p.type("#composer-text", "longer history");
  await p.click("#composer-send");
  await new Promise((r) => setTimeout(r, 500));
  for (let i = 0, n = -1; i < 15 && n !== await p.eval(rowCount); i++) {
    n = await p.eval(rowCount);
    await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: -400 })); window.scrollTo(0, 0)");
    await p.until(`${rowCount} > ${n}`, 1500);
    await new Promise((r) => setTimeout(r, 300));
  }
  const capped = await p.eval(rowCount);
  check(capped >= 1000 && capped < 1100, `earlier: auto-loading stops at the soft cap (1000 rows, plus at most the last page: ${capped})`);
  await p.click("#load-earlier");
  check(await p.until(`${rowCount} > ${capped}`, 4000) === true, `earlier: past the cap, Load earlier still loads (${await p.eval(rowCount)})`);
  // A failed load says so; the notice and Try again leave the row being read where it is. Sends of loadEarlier fail
  // while window.__failEarlier > 0 (they ask for a row the hub does not have); window.__holdEarlier holds the next one 600 ms;
  // window.__parkEarlier holds the next one until window.__parked() (a log read that ends while the reader still scrolls);
  // window.__slowEarlier holds each one that many ms (the hub's read of a long log).
  const readRow = `(() => { const e = [...document.querySelectorAll('.items > .msg')].find((e) => e.getBoundingClientRect().bottom > 0);
    return JSON.stringify(e ? { text: e.textContent, top: e.getBoundingClientRect().top } : { text: "", top: NaN }); })()`;
  const topOf = (text) => `[...document.querySelectorAll('.items > .msg')].find((e) => e.textContent === ${JSON.stringify(text)})?.getBoundingClientRect().top ?? NaN`;
  const failNotice = "document.body.innerText.includes('Could not load earlier messages.')";
  await p.eval(`(() => { const send = WebSocket.prototype.send; window.__failEarlier = 0;
    WebSocket.prototype.send = function (d) {
      if (window.__failEarlier > 0 && typeof d === 'string' && d.includes('"loadEarlier"')) { window.__failEarlier--; d = d.replace(/"before":"[^"]*"/, '"before":"no-such-row"'); }
      if (window.__holdEarlier && typeof d === 'string' && d.includes('"loadEarlier"')) { window.__holdEarlier = 0; setTimeout(() => send.call(this, d), 600); return; }
      if (window.__slowEarlier && typeof d === 'string' && d.includes('"loadEarlier"')) { setTimeout(() => send.call(this, d), window.__slowEarlier); return; }
      return send.call(this, d);
    }; })()`);
  await installEarlierParking(p);
  await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: -400 })); window.scrollTo(0, 3000)");
  await new Promise((r) => setTimeout(r, 600));
  const read0 = JSON.parse(await p.eval(readRow));
  await p.eval("window.__failEarlier = 1; document.getElementById('load-earlier').click()");
  check(await p.until(failNotice, 4000) === true, "earlier: a failed load says so");
  await new Promise((r) => setTimeout(r, 300));
  const read1 = await p.eval(topOf(read0.text));
  check(Math.abs(read1 - read0.top) < 2, `earlier: the failure notice leaves the row being read in place (${Math.round(read0.top)} -> ${Math.round(read1)})`);
  const f0 = await p.eval(rowCount);
  const retry0 = await p.eval(topOf(read0.text));
  await p.eval("window.__holdEarlier = 1; document.getElementById('load-earlier').click()");
  await new Promise((r) => setTimeout(r, 250));
  const busy = await p.eval(`!${failNotice} && !!document.querySelector('.earlier-pill') && document.getElementById('load-earlier')?.disabled === true`);
  const readBusy = await p.eval(topOf(read0.text));
  check(busy === true && Math.abs(readBusy - retry0) < 2, `earlier: while it tries again, the row being read stays in place (${Math.round(retry0)} -> ${Math.round(readBusy)})`);
  await p.until(`${rowCount} > ${f0} && !${failNotice}`, 4000);
  await new Promise((r) => setTimeout(r, 300));
  const read2 = await p.eval(topOf(read0.text));
  check(await p.eval(rowCount) > f0 && Math.abs(read2 - retry0) < 2, `earlier: Try again leaves the row being read in place (${Math.round(retry0)} -> ${Math.round(read2)})`);
  // Another session in the pane: a script's scroll to the top (opening, switching) loads nothing; the reader's does.
  hub.stdin.write("session w2:p2 s8 300\n");
  await p.until("document.body.innerText.includes('New session reply 299')", 4000);
  await new Promise((r) => setTimeout(r, 900));
  const s0 = await p.eval(rowCount);
  await p.eval("window.scrollTo(0, 0)");
  await new Promise((r) => setTimeout(r, 900));
  check(await p.eval(rowCount) === s0, `earlier: a script's scroll to the top loads nothing (${s0} -> ${await p.eval(rowCount)})`);
  await p.eval("window.scrollTo(0, document.documentElement.scrollHeight)");
  await new Promise((r) => setTimeout(r, 300));
  await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: -400 })); window.scrollTo(0, 0)");
  check(await p.until(`${rowCount} > ${s0}`, 4000) === true, `earlier: the reader's scroll to the top loads (${s0} -> ${await p.eval(rowCount)})`);
  // A failure in one session is not another's: back to the first session, no notice, and the button loads.
  await p.eval("window.__failEarlier = 1; document.getElementById('load-earlier').click()");
  await p.until(failNotice, 4000);
  hub.stdin.write("session w2:p2 s4\n");
  await p.until("!document.body.innerText.includes('New session reply 299')", 4000);
  await new Promise((r) => setTimeout(r, 900));
  check(await p.eval(`!${failNotice} && document.getElementById('load-earlier')?.textContent === 'Load earlier'`) === true,
    `earlier: another session in the pane starts without the other's failure (${await p.eval("document.getElementById('load-earlier')?.textContent")})`);
  phase("earlier history");
  // The reader's case: older rows that come while the reader still scrolls at the top wait until the scroll stops. On a
  // phone a finger on the glass (or the bounce at the top), on a Mac a trackpad's momentum, takes the page back to its
  // own top whatever the page does meanwhile: rows put in then push the message being read out of view. Rows of a long
  // session that come from the hub; the reply to the read waits until window.__parked().
  hub.stdin.write("session w2:p2 s12 60\nrich s12 600\n");
  await p.until("document.body.innerText.includes('New session reply 59')", 4000);
  await new Promise((r) => setTimeout(r, 900));
  // While the pill shows, it is the one "Loading earlier": the Load earlier button keeps its room but is not seen.
  // An XPath query: a walk of every node here, on the slowed CPU, would be a stall of its own.
  const loadingSeen = `(() => { const r = document.evaluate("//body//*[text()[contains(., 'Loading earlier')]]", document, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);
    let n = 0; for (let i = 0; i < r.snapshotLength; i++) { const e = r.snapshotItem(i); if (e.getClientRects().length > 0 && getComputedStyle(e).visibility !== 'hidden') n++; } return n; })()`;
  const loadButtonSeen = "(() => { const b = document.getElementById('load-earlier'); return !!b && getComputedStyle(b).visibility !== 'hidden'; })()";
  /** A reader's scroll to the top, a read of older rows held, and the row on top then. */
  phase("earlier held gestures");
  async function heldAtTop() { return parkEarlierAtTop(p, readRow); }
  const finger = await heldAtTop(), fingerN = await p.eval(rowCount);
  await p.cdp("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x: 195, y: 420 }] });
  await p.eval("window.__parked()");
  await new Promise((r) => setTimeout(r, 700));
  const fingerMid = await p.eval(topOf(finger.text)), fingerMidN = await p.eval(rowCount), fingerPill = await p.eval("!!document.querySelector('.earlier-pill')");
  check(fingerMidN === fingerN && Math.abs(fingerMid - finger.top) <= 2 && fingerPill,
    `earlier: while a finger is on the glass at the top, older rows wait and the row being read stays (${fingerN} -> ${fingerMidN} rows, ${Math.round(finger.top)} -> ${Math.round(fingerMid)}, notice ${fingerPill})`);
  const fingerSeen = await p.eval(loadingSeen), fingerButton = await p.eval(loadButtonSeen);
  check(fingerSeen === 1 && !fingerButton,
    `earlier: while a finger is on the glass and older rows wait, one notice shows: the pill, not the button under it (${fingerSeen}, button seen ${fingerButton})`);
  await p.shot("earlier-held");
  await p.cdp("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await p.until(`${rowCount} > ${fingerN}`, 3000);
  await new Promise((r) => setTimeout(r, 800));
  const fingerAfter = await p.eval(topOf(finger.text)), fingerAfterN = await p.eval(rowCount);
  check(fingerAfterN > fingerN && Math.abs(fingerAfter - finger.top) <= 2,
    `earlier: when the finger lifts they go in, and the row being read stays (${fingerN} -> ${fingerAfterN} rows, ${Math.round(finger.top)} -> ${Math.round(fingerAfter)})`);
  await p.shot("earlier-held-in");
  // A render while the finger is down puts a new node in place of the one under it (a reply comes in): the lift goes to
  // the old node, out of the page, and never reaches the window. The rows still go in when the finger lifts.
  const swap = await heldAtTop(), swapN = await p.eval(rowCount);
  await p.cdp("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x: 195, y: 420 }] });
  const swapped = await p.eval("(() => { const e = document.elementFromPoint(195, 420); if (!e || !e.closest('.items')) return false; e.replaceWith(e.cloneNode(true)); return true; })()");
  await p.eval("window.__parked()");
  await new Promise((r) => setTimeout(r, 700));
  const swapMidN = await p.eval(rowCount);
  await p.cdp("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await p.until(`${rowCount} > ${swapN}`, 3000);
  await new Promise((r) => setTimeout(r, 800));
  const swapAfter = await p.eval(topOf(swap.text)), swapAfterN = await p.eval(rowCount), swapPill = await p.eval("!!document.querySelector('.earlier-pill')");
  check(swapped && swapMidN === swapN && swapAfterN > swapN && Math.abs(swapAfter - swap.top) <= 2 && !swapPill,
    `earlier: the node under the finger leaves the page while it waits; at the lift the rows go in and the notice goes (node swapped ${swapped}, ${swapN} -> ${swapMidN} -> ${swapAfterN} rows, ${Math.round(swap.top)} -> ${Math.round(swapAfter)}, notice ${swapPill})`);
  // The same swap, then the finger keeps moving for more than 10 s: the moves go to the old node too, so no time limit
  // takes the finger for lifted. The rows wait until the lift.
  const drag = await heldAtTop(), dragN = await p.eval(rowCount);
  await p.cdp("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x: 195, y: 420 }] });
  const dragSwapped = await p.eval("(() => { const e = document.elementFromPoint(195, 420); if (!e || !e.closest('.items')) return false; e.replaceWith(e.cloneNode(true)); return true; })()");
  await p.eval("window.__parked()");
  for (let t = 0; t < 11000; t += 200) {
    await p.cdp("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ x: 195, y: (t / 200) % 2 ? 424 : 420 }] });
    await new Promise((r) => setTimeout(r, 200));
  }
  const dragMidN = await p.eval(rowCount), dragMid = await p.eval(topOf(drag.text));
  await p.cdp("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await p.until(`${rowCount} > ${dragN}`, 3000);
  await new Promise((r) => setTimeout(r, 800));
  const dragAfterN = await p.eval(rowCount), dragAfter = await p.eval(topOf(drag.text));
  check(dragSwapped && dragMidN === dragN && dragAfterN > dragN && Math.abs(dragAfter - drag.top) <= 2,
    `earlier: the node under the finger leaves the page and the finger moves on for 11 s: the rows wait for the lift (node swapped ${dragSwapped}, ${dragN} -> ${dragMidN} -> ${dragAfterN} rows, ${Math.round(drag.top)} -> ${Math.round(dragMid)} -> ${Math.round(dragAfter)})`);
  // Two fingers: the node under finger A leaves the page, then finger B goes down on another node. B lifts first (A is
  // still down), then A: A's lift goes to its old node only, out of the page. The rows go in at A's lift. The browser
  // lifts every point of the protocol's touchEnd at once, so the two lifts are sent from the page, each to its node.
  const two = await heldAtTop(), twoN = await p.eval(rowCount);
  // A goes down on a node inside a row, not on a row itself: the page keeps each row's node and puts it back.
  const ay = await p.eval(`(() => { for (let y = 420; y < 700; y += 4) { const e = document.elementFromPoint(195, y);
    if (e && e.closest('.items') && !e.matches('.items > *')) return y; } return 420; })()`);
  await p.cdp("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ id: 1, x: 195, y: ay }] });
  const twoSwapped = await p.eval(`(() => { const e = document.elementFromPoint(195, ${ay}); if (!e || !e.closest('.items')) return false;
    window.__a = e; e.replaceWith(e.cloneNode(true)); return true; })()`);
  const twoB = await p.eval(`(() => { const a = document.elementFromPoint(195, ${ay}); for (let y = 120; y < 700; y += 20) {
    const b = document.elementFromPoint(195, y); if (b && b.closest('.items') && !b.contains(a) && !a.contains(b) && b.closest('.msg, .logged, .tools') !== a.closest('.msg, .logged, .tools')) return y; } return 0; })()`);
  await p.cdp("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ id: 1, x: 195, y: ay }, { id: 2, x: 195, y: twoB || 250 }] });
  await p.eval("window.__parked()");
  await new Promise((r) => setTimeout(r, 300));
  const lift = (at, id, down) => `(() => { const at = ${at}, t = (n, e) => new Touch({ identifier: n, target: e, clientX: 195, clientY: 420 });
    at.dispatchEvent(new TouchEvent('touchend', { bubbles: true, touches: ${down}, changedTouches: [t(${id}, at)] })); return at.isConnected; })()`;
  const bOn = await p.eval(lift(`document.elementFromPoint(195, ${twoB || 250})`, 2, "[t(1, window.__a)]"));   // B lifts
  await new Promise((r) => setTimeout(r, 700));
  const twoMidN = await p.eval(rowCount);
  const aOn = await p.eval(lift("window.__a", 1, "[]"));   // A lifts, on its old node
  await p.until(`${rowCount} > ${twoN}`, 3000);
  await new Promise((r) => setTimeout(r, 800));
  const twoAfterN = await p.eval(rowCount), twoAfter = await p.eval(topOf(two.text));
  await p.cdp("Input.dispatchTouchEvent", { type: "touchCancel", touchPoints: [] });   // the protocol's points up too
  check(twoSwapped && twoB > 0 && bOn && !aOn && twoMidN === twoN && twoAfterN > twoN && Math.abs(twoAfter - two.top) <= 2,
    `earlier: two fingers, A's node leaves the page, B lifts first, then A: the rows go in at A's lift (node swapped ${twoSwapped}, B at ${twoB}, B on page ${bOn}, A on page ${aOn}, ${twoN} -> ${twoMidN} -> ${twoAfterN} rows, ${Math.round(two.top)} -> ${Math.round(twoAfter)})`);
  // A hand that keeps pushing at the top (a trackpad's rubber band): wheel ticks every 40 ms while the rows come (a
  // trackpad sends 60 or more a second). From a timer in the page: a tick sent from here waits for the page's reply,
  // and on a busy machine two of them can be more than the round's 150 ms apart.
  const push = await heldAtTop(), pushN = await p.eval(rowCount);
  await p.eval("window.__push = setInterval(() => window.dispatchEvent(new WheelEvent('wheel', { deltaY: -60 })), 40)");
  await new Promise((r) => setTimeout(r, 100));
  await p.eval("window.__parked()");   // the rows come once the pushing has started
  await new Promise((r) => setTimeout(r, 700));
  const pushMidN = await p.eval(rowCount), pushMid = await p.eval(topOf(push.text));
  await p.eval("clearInterval(window.__push)");
  check(pushMidN === pushN && Math.abs(pushMid - push.top) <= 2,
    `earlier: while the reader keeps pushing at the top, older rows wait (${pushN} -> ${pushMidN} rows, ${Math.round(push.top)} -> ${Math.round(pushMid)})`);
  await p.until(`${rowCount} > ${pushN}`, 3000);
  await new Promise((r) => setTimeout(r, 800));
  const pushAfter = await p.eval(topOf(push.text)), pushAfterN = await p.eval(rowCount);
  check(pushAfterN > pushN && Math.abs(pushAfter - push.top) <= 2,
    `earlier: when the pushing stops they go in, and the row being read stays (${pushN} -> ${pushAfterN} rows, ${Math.round(push.top)} -> ${Math.round(pushAfter)})`);
  // A fast fling up, from 3000 px below the top at 6000 px/s, slowing down at 6000 px/s² (it would stop at the top), each
  // read 300 ms on its way (the hub's read of a 30 MB log took 85-300 ms, longer as more of it is loaded): older rows load early enough to go in while the page still glides, far from the top, and the glide goes on over them; the
  // rows in view move only with the glide (2 px at most). Once with a trackpad's momentum (wheel ticks, on a Mac), once
  // with a finger's glide (scroll alone after the lift, on a phone). The glide moves from where the page is at each step,
  // as the browser's does. `window.__fling()` gives: where the read began (px above the top of the view), how long it took
  // to the rows in, where the view was then, whether the glide met the top before, the worst move of the rows in view
  // past the glide's own, and the longest step.
  await p.eval(`window.__fling = (wheel) => new Promise((done) => {
    const rows = () => document.querySelectorAll('.items > .msg').length, n0 = rows();
    const at = [...document.querySelectorAll('.items > .msg')].find((e) => e.getBoundingClientRect().top > 0), y0 = at.getBoundingClientRect().top;
    const out = { from: -1, loadMs: -1, inAt: -1, hitTop: false, hitBeforeIn: false, drift: 0, stall: 0, rows: 0 };
    let v = 6000, last = performance.now(), glided = 0, t0 = last, asked = 0, prevY = scrollY;
    const seen = new MutationObserver(() => { if (out.from < 0 && document.querySelector('.earlier-pill')) { out.from = Math.round(scrollY); asked = performance.now(); } });
    seen.observe(document.body, { childList: true, subtree: true });
    const step = () => {
      const t = performance.now(), dt = Math.min(t - last, 50) / 1000; out.stall = Math.max(out.stall, t - last); last = t;
      if (out.inAt < 0 && rows() > n0) { out.inAt = Math.round(prevY); out.loadMs = Math.round(t - (asked || t0)); window.__flingIn = true; }
      const dy = Math.min(v * dt, scrollY);
      if (wheel) window.dispatchEvent(new WheelEvent('wheel', { deltaY: -dy }));
      const was = scrollY; window.scrollBy(0, -dy); glided += was - scrollY;
      out.drift = Math.max(out.drift, Math.abs(at.getBoundingClientRect().top - (y0 + glided)));
      if (scrollY <= 0) { out.hitTop = true; if (out.inAt < 0) out.hitBeforeIn = true; }
      prevY = scrollY;
      v -= 6000 * dt;
      if (v > 0 && !out.hitTop) setTimeout(step, 16);
      else { seen.disconnect(); out.drift = Math.round(out.drift); out.stall = Math.round(out.stall); out.rows = rows() - n0; done(out); }
    };
    setTimeout(step, 16);
  })`);
  await p.eval("window.__slowEarlier = 300");
  for (const [s, wheel] of [["s14", true], ["s15", false]]) {
    hub.stdin.write(`session w2:p2 ${s} 120\nrich ${s} 600\n`);
    await p.until("document.body.innerText.includes('New session reply 119')", 4000);
    await new Promise((r) => setTimeout(r, 900));
    await p.eval("window.scrollTo(0, 3000)");
    await new Promise((r) => setTimeout(r, 600));
    if (wheel) await p.shot("earlier-fling-start");
    if (!wheel) { await p.cdp("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x: 195, y: 420 }] }); await p.cdp("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] }); }
    await p.eval("window.__flingIn = false; window.__flingOut = null; window.__fling(" + wheel + ").then((o) => { window.__flingOut = JSON.stringify(o); })");
    // Mid-glide, the older rows in: the page still glides while the shot is taken.
    if (wheel && await p.until("window.__flingIn", 3000)) await p.shot("earlier-fling-mid");
    const f = JSON.parse(await p.until("window.__flingOut", 5000));
    await new Promise((r) => setTimeout(r, 1200));
    const how = wheel ? "a trackpad's fling" : "a finger's fling";
    check(f.from > 0 && f.inAt > 0 && !f.hitBeforeIn && !f.hitTop && f.drift <= 2,
      `earlier: ${how} up, fast: older rows go in before the top and the glide goes on over them (read from ${f.from} px, rows in after ${f.loadMs} ms, ${f.inAt} px from the top just before, +${f.rows} rows, top met ${f.hitTop}${f.hitBeforeIn ? " before the rows" : ""}, drift ${f.drift} px, longest step ${f.stall} ms)`);
    if (wheel) await p.shot("earlier-fling");
  }
  // The read's answer comes while the finger is still on the glass (it drags up fast, then rests), and the finger then
  // flicks: at the lift the rows go in, while the page glides far from the top, not when the glide ends at the top.
  hub.stdin.write("session w2:p2 s16 120\nrich s16 600\n");
  await p.until("document.body.innerText.includes('New session reply 119')", 4000);
  await new Promise((r) => setTimeout(r, 900));
  await p.eval("window.scrollTo(0, 3000)");
  await new Promise((r) => setTimeout(r, 600));
  await p.eval("window.__parkEarlier = 1; window.__parked = undefined");
  await p.cdp("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x: 195, y: 420 }] });
  const dragFrom = await p.eval(`new Promise((done) => {
    let last = performance.now(); const t0 = last;
    const step = () => {
      const t = performance.now(), dt = Math.min(t - last, 50) / 1000; last = t;
      if (typeof window.__parked === 'function' || t - t0 > 1500 || scrollY < 600) { done(Math.round(scrollY)); return; }
      window.scrollBy(0, -6000 * dt); setTimeout(step, 16);
    };
    setTimeout(step, 16);
  })`);
  const asked = await p.until("typeof window.__parked === 'function'", 2000) === true;
  const heldN = await p.eval(rowCount);
  await p.eval("window.__parked()");
  await new Promise((r) => setTimeout(r, 400));
  const heldStill = await p.eval(rowCount);
  await p.cdp("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  const g = JSON.parse(await p.eval("window.__fling(false).then(JSON.stringify)"));
  await new Promise((r) => setTimeout(r, 1200));
  check(asked && dragFrom > 600 && heldStill === heldN && g.inAt > 0 && !g.hitBeforeIn && !g.hitTop && g.drift <= 2,
    `earlier: the answer comes while the finger is down, then a flick: the rows wait for the lift and go in as the glide begins (read asked at ${dragFrom} px, ${heldN} -> ${heldStill} rows with the finger down, then in at ${g.inAt} px from the top, +${g.rows} rows, top met ${g.hitTop}${g.hitBeforeIn ? " before the rows" : ""}, drift ${g.drift} px)`);
  await p.eval("window.__slowEarlier = 0");
  // Older rows of a real reply's weight, on a CPU slowed 4x (a phone): near the top a "Loading earlier" notice shows over
  // the rows (it moves none of them) until the rows are in, and the page never stalls past the budget. The meter: the
  // longest gap between ticks of a 5 ms timer (the Mac churn check's meter). WEB_PROFILE=<dir>: a CPU profile of the load.
  // Budget: all of a page put in at once, the page laid every row out again, stalled 127-137 ms here; rows a few per
  // frame, the others left in place, 18-61 ms over 12 runs on a busy machine. 80 sits between the two.
  const STALL_BUDGET = 80;
  hub.stdin.write("session w2:p2 s10 120\nrich s10 400\n");
  await p.until("document.body.innerText.includes('New session reply 119')", 4000);
  await new Promise((r) => setTimeout(r, 900));
  await p.cdp("Emulation.setCPUThrottlingRate", { rate: 4 });
  await p.eval(`(() => { window.__gap = 0; let last = performance.now();
    clearInterval(window.__meter); window.__meter = setInterval(() => { const n = performance.now(); window.__gap = Math.max(window.__gap, n - last); last = n; }, 5); })()`);
  const pill = `(() => { const e = document.querySelector('.earlier-pill'); if (!e) return null; const r = e.getBoundingClientRect(), s = getComputedStyle(e);
    return r.height > 0 && r.top >= 0 && r.bottom <= innerHeight && s.visibility !== 'hidden' && Number(s.opacity) > 0.5 ? Math.round(r.top) : null; })()`;
  const rich0 = await p.eval(rowCount);
  if (process.env.WEB_PROFILE) { await p.cdp("Profiler.enable"); await p.cdp("Profiler.setSamplingInterval", { interval: 200 }); await p.cdp("Profiler.start"); }
  await p.eval("window.__holdEarlier = 1; window.__gap = 0");
  const richTop = JSON.parse(await p.eval(`(() => { window.dispatchEvent(new WheelEvent('wheel', { deltaY: -400 })); window.scrollTo(0, 0); return ${readRow}; })()`));
  await new Promise((r) => setTimeout(r, 250));
  const pillAt = await p.eval(pill), richBusy = await p.eval(topOf(richTop.text));
  check(pillAt !== null && Math.abs(richBusy - richTop.top) < 2,
    `earlier: while older rows load, a notice shows at the top (at ${pillAt}) and the row being read stays (${Math.round(richTop.top)} -> ${Math.round(richBusy)})`);
  // One notice: the Load earlier button keeps its words while the pill says it.
  const seen = await p.eval(loadingSeen);
  const buttonSeen = await p.eval(loadButtonSeen);
  check(seen === 1 && !buttonSeen, `earlier: while older rows load, one notice shows: the pill, not the button under it (${seen}, button seen ${buttonSeen})`);
  await p.shot("earlier-loading");
  await p.shot("earlier-loading", true);
  // Each frame while the rows go in (a few per frame), before it is painted: the row being read never moves.
  await p.eval(`(() => { window.__moved = 0; window.__looks = 0; const at = ${richTop.top};
    const look = () => { const t = ${topOf(richTop.text)}; if (!Number.isNaN(t)) { window.__moved = Math.max(window.__moved, Math.abs(t - at)); window.__looks++; }
      if (window.__looking) requestAnimationFrame(look); };
    window.__looking = true; requestAnimationFrame(look); })()`);
  check(await p.until(`${rowCount} >= ${rich0 + 100}`, 8000) === true, `earlier: heavy older rows load (${rich0} -> ${await p.eval(rowCount)})`);
  await new Promise((r) => setTimeout(r, 1500));
  if (process.env.WEB_PROFILE) {
    const { profile } = await p.cdp("Profiler.stop");
    writeFileSync(join(process.env.WEB_PROFILE, "earlier.cpuprofile"), JSON.stringify(profile));
  }
  const richMoved = await p.eval("window.__looking = false; window.__moved"), looks = await p.eval("window.__looks");
  check(looks > 10 && richMoved < 2, `earlier: while the rows go in, the row being read never moves (worst ${Math.round(richMoved)} px over ${looks} frames)`);
  const gap = await p.eval("window.__gap"), richAfter = await p.eval(topOf(richTop.text));
  check(await p.eval(pill) === null, "earlier: the notice goes when the rows are in");
  check(Math.abs(richAfter - richTop.top) < 2, `earlier: after the heavy load, the row being read stays in place (${Math.round(richTop.top)} -> ${Math.round(richAfter)})`);
  check(gap < STALL_BUDGET, `earlier: loading heavy older rows stalls the page at most ${Math.round(gap)} ms (CPU 4x slower; budget ${STALL_BUDGET})`);
  // Back to the newest rows while older rows still go in: the page stays at the bottom (the rest goes in above).
  const endGap = "Math.round(document.documentElement.scrollHeight - window.innerHeight - window.scrollY)";
  const rich1 = await p.eval(rowCount);
  await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: -400 })); window.scrollTo(0, 0)");
  await p.until(`${rowCount} > ${rich1}`, 4000);
  const midway = await p.eval(`${rowCount} < ${rich1 + 100}`);
  await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: 400 })); window.scrollTo(0, document.documentElement.scrollHeight)");
  await p.until(`${rowCount} >= ${rich1 + 100}`, 8000);
  await new Promise((r) => setTimeout(r, 800));
  check(midway === true && await p.eval(`${endGap} <= 4`) === true,
    `earlier: back at the newest rows mid-load, the page stays at the bottom (midway ${midway}, gap ${await p.eval(endGap)})`);
  await p.eval("clearInterval(window.__meter)");
  await p.cdp("Emulation.setCPUThrottlingRate", { rate: 1 });
  hub.stdin.write("session w2:p2 s4\n");
  await p.until("!document.body.innerText.includes('New session reply 119')", 4000);
  await new Promise((r) => setTimeout(r, 900));
  await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: 400 })); window.scrollTo(0, document.documentElement.scrollHeight)");
  await p.type("#composer-text", "ask me");
  await p.click("#composer-send");
  check(await p.until("(document.querySelector('.card.question')?.textContent ?? '').includes('Answer this question on the Mac.')") === true,
    "question: a screen-read question says answer on the Mac");
  check(await p.eval("!document.querySelector('#submit-answer') && !document.querySelector('.option')") === true, "question: no answer controls for it");
  await p.shot("question-mac");

  phase("newest message and following");
  // The newest message: a conversation opens at the bottom and stays there while rows arrive, after a switch, a reload
  // and a return from the background. A reader who scrolled up stays where they are.
  const bottomGap = "Math.round(document.documentElement.scrollHeight - window.innerHeight - window.scrollY)";
  const atEnd = `${bottomGap} <= 4`;
  const grow = (session, n) => hub.stdin.write(`grow ${session} ${n}\n`);
  const rows = "document.querySelectorAll('.msg').length";
  await p.eval("location.hash = '#pane=w2%3Ap1'");
  await p.until(`${rows} > 0`);
  const before = await p.eval(rows);
  grow("s3", 30);   // the agent writes after the page opened
  await p.until(`${rows} >= ${before + 30}`);
  check(await p.until(atEnd, 2000) === true, `newest: rows that arrive after opening -> at the bottom (gap ${await p.eval(bottomGap)})`);
  grow("s3", 3);
  await p.until(`${rows} >= ${before + 33}`);
  check(await p.until(atEnd, 2000) === true, `newest: more rows while at the bottom -> still there (gap ${await p.eval(bottomGap)})`);
  await p.until("!document.querySelector('.toast')", 8000);   // an earlier check's message would cover the end
  await p.shot("newest");
  await p.shot("newest", true);
  // Scrolled up: rows arrive, the reader is not pulled back.
  await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: -100 })); window.scrollTo(0, 200)");
  await new Promise((r) => setTimeout(r, 300));
  grow("s3", 3);
  await p.until(`${rows} >= ${before + 36}`);
  await new Promise((r) => setTimeout(r, 500));
  check(await p.eval("Math.round(window.scrollY)") === 200, `newest: scrolled up -> stays (y ${await p.eval("Math.round(window.scrollY)")})`);
  // Another agent, then back: each opens at its newest message.
  grow("s4", 30);
  await p.eval("location.hash = '#pane=w2%3Ap2'");
  await p.until(`${rows} >= 30`);
  check(await p.until(atEnd, 2000) === true, `newest: switch to another agent -> at the bottom (gap ${await p.eval(bottomGap)})`);
  await p.eval("location.hash = '#pane=w2%3Ap1'");
  await p.until(`${rows} >= ${before + 36}`);
  check(await p.until(atEnd, 2000) === true, `newest: back to the first agent -> at the bottom (gap ${await p.eval(bottomGap)})`);
  // Reload. From here on the page keeps its sockets where the check can close one (a dropped connection).
  await p.onEveryLoad("{ const W = WebSocket; window.__sockets = []; window.WebSocket = class extends W { constructor(...a) { super(...a); window.__sockets.push(this); } }; }");
  await p.eval("window.__loaded = true");
  await p.reload();
  await p.until("!window.__loaded && window.__sockets", 8000);
  await p.until(`${rows} >= ${before + 36}`, 8000);
  check(await p.until(atEnd, 2000) === true, `newest: reload -> at the bottom (gap ${await p.eval(bottomGap)})`);
  // The connection drops while the reader is scrolled up: the short "Not connected" screen puts the page at the top. That
  // is not the reader scrolling: back online, rows arrive and the reader is not pulled to the bottom.
  const y = "Math.round(window.scrollY)";
  await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: -100 })); window.scrollTo(0, 200)");
  await new Promise((r) => setTimeout(r, 300));
  await p.eval("window.__sockets.at(-1).close()");
  await p.until("document.body.innerText.includes('Not connected')");
  await p.until(`${rows} >= ${before + 36}`, 8000);
  grow("s3", 2);
  await p.until(`${rows} >= ${before + 38}`);
  await new Promise((r) => setTimeout(r, 500));
  check(await p.eval(bottomGap) > 200, `newest: connection dropped while scrolled up -> not pulled to the bottom after it returns (gap ${await p.eval(bottomGap)}, y ${await p.eval(y)})`);
  // Pinch zoom: what the reader sees is the visual viewport. At the bottom of the page but zoomed in on its top half,
  // the newest rows are out of view: rows that arrive do not move the page.
  await p.eval("window.scrollTo(0, document.documentElement.scrollHeight)");
  await new Promise((r) => setTimeout(r, 300));
  await p.cdp("Emulation.setPageScaleFactor", { pageScaleFactor: 2 });
  await p.until("window.visualViewport.scale > 1.5");
  // The reader moves only the zoomed view (the page itself stays at its bottom: no window scroll event).
  const zoomedY = await p.eval(y);
  await p.cdp("Input.dispatchMouseEvent", { type: "mouseWheel", x: 195, y: 400, deltaX: 0, deltaY: 120 });
  await p.until("window.visualViewport.offsetTop > 50");
  const moved = await p.eval("Math.round(window.visualViewport.offsetTop)");
  await new Promise((r) => setTimeout(r, 300));
  grow("s3", 2);
  await p.until(`${rows} >= ${before + 40}`);
  await new Promise((r) => setTimeout(r, 500));
  check(moved > 50 && await p.eval(y) === zoomedY, `newest: zoomed in above the newest rows -> stays (view moved ${moved}, y ${zoomedY} -> ${await p.eval(y)})`);
  await p.cdp("Emulation.setPageScaleFactor", { pageScaleFactor: 1 });
  await p.until("window.visualViewport.scale < 1.1");
  await p.eval("window.scrollTo(0, document.documentElement.scrollHeight)");   // following again
  await new Promise((r) => setTimeout(r, 300));
  // The reader scrolls up in the same frame as the page grew (before the resize observer saw the new height): their
  // scroll counts, the page is not pulled back.
  await p.eval(`(() => { const d = document.createElement('div'); d.style.height = '600px'; document.getElementById('app').append(d);
    window.dispatchEvent(new WheelEvent('wheel', { deltaY: -300 })); window.scrollBy(0, -300); })()`);
  await new Promise((r) => setTimeout(r, 900));   // past the moment after the reader's input
  check(await p.eval(bottomGap) > 300, `newest: scrolled up while the page grew -> stays (gap ${await p.eval(bottomGap)})`);
  await p.eval("window.scrollTo(0, document.documentElement.scrollHeight)");   // following again
  await new Promise((r) => setTimeout(r, 300));
  // The scroll bar held longer than that moment, then dragged up in the frame the page grew: still the reader.
  await p.eval("document.documentElement.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))");
  await new Promise((r) => setTimeout(r, 900));
  await p.eval(`(() => { const d = document.createElement('div'); d.style.height = '600px'; document.getElementById('app').append(d);
    window.scrollBy(0, -300); })()`);
  await new Promise((r) => setTimeout(r, 900));
  check(await p.eval(bottomGap) > 300, `newest: scroll bar held, dragged up while the page grew -> stays (gap ${await p.eval(bottomGap)})`);
  await p.eval("document.documentElement.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }))");
  await p.eval("window.scrollTo(0, document.documentElement.scrollHeight)");
  await new Promise((r) => setTimeout(r, 300));
  // The scroll bar let go outside the window (no pointerup reaches the page): the window losing focus ends the hold, and
  // a hold with no movement for 10 s ends by itself. Following again either way.
  const grows = "(() => { const d = document.createElement('div'); d.style.height = '400px'; document.getElementById('app').append(d); })()";
  await p.eval("document.documentElement.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true })); window.dispatchEvent(new Event('blur'))");
  await new Promise((r) => setTimeout(r, 900));
  await p.eval(grows);
  check(await p.until(atEnd, 2000) === true, `newest: scroll bar let go outside the window -> follows again (gap ${await p.eval(bottomGap)})`);
  // The page's own scroll to the bottom is delivered late: the page grows, the page scrolls to its end, and in the same
  // frame the scroll bar is pressed and the page grows again, so that scroll event arrives inside the hold, above the new
  // end. It is the page's, not the reader's; the hold is never let go and nothing else happens. After the 10 s limit the
  // page is at the newest rows. (A second resize observer runs after the page's, which is the one that scrolls.)
  await p.eval(`(() => { const ro = new ResizeObserver(() => { ro.disconnect();
    document.documentElement.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true })); ${grows}; });
    ro.observe(document.body); ${grows}; })()`);
  await new Promise((r) => setTimeout(r, 10500));
  check(await p.until(atEnd, 2000) === true, `newest: own scroll delivered inside a still hold, never let go -> at the bottom after 10 s (gap ${await p.eval(bottomGap)})`);
  // The same late scroll, then the hand moves a little on the held bar without scrolling, and lets go: the hold had a move,
  // so only knowing the page's own scroll keeps it following.
  await p.eval(`(() => { const ro = new ResizeObserver(() => { ro.disconnect();
    document.documentElement.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true })); ${grows}; });
    ro.observe(document.body); ${grows}; })()`);
  await new Promise((r) => setTimeout(r, 200));
  await p.eval("document.documentElement.dispatchEvent(new PointerEvent('pointermove', { bubbles: true }))");
  await new Promise((r) => setTimeout(r, 300));
  await p.eval("document.documentElement.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }))");
  check(await p.until(atEnd, 2000) === true, `newest: own scroll delivered inside a hold, a move without scrolling, let go -> follows (gap ${await p.eval(bottomGap)})`);
  // Pressed and let go, the page did not move: following as before the press.
  await p.eval("document.documentElement.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))");
  await new Promise((r) => setTimeout(r, 300));
  await p.eval("document.documentElement.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }))");
  await new Promise((r) => setTimeout(r, 400));
  await p.eval(grows);
  check(await p.until(atEnd, 2000) === true, `newest: pressed and let go, the page did not move -> follows as before (gap ${await p.eval(bottomGap)})`);
  // The reader scrolls up in steps while rows keep arriving between the steps (an agent writing fast): nothing pulls the
  // page down during the scrolling, so the reader gets away from the bottom.
  for (let i = 0; i < 6; i++) {
    await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: -150 })); window.scrollBy(0, -150)");
    await new Promise((r) => setTimeout(r, 40));
    await p.eval(grows);
    await new Promise((r) => setTimeout(r, 40));
  }
  await new Promise((r) => setTimeout(r, 900));
  await p.eval(grows);
  await new Promise((r) => setTimeout(r, 500));
  check(await p.eval(bottomGap) > 600, `newest: scrolled up in steps while rows arrive -> gets away from the bottom (gap ${await p.eval(bottomGap)})`);
  await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: 400 })); window.scrollTo(0, document.documentElement.scrollHeight)");
  await new Promise((r) => setTimeout(r, 900));
  // The scroll bar held still while a scroll key moves the page (or a click on the bar's track): that is the reader.
  // PageUp: not following after the hold ends.
  const key = (k) => `document.dispatchEvent(new KeyboardEvent('keydown', { key: '${k}', bubbles: true }))`;
  await p.eval(`document.documentElement.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true })); ${key("PageUp")}; window.scrollBy(0, -400)`);
  await new Promise((r) => setTimeout(r, 300));
  await p.eval("document.documentElement.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }))");
  await new Promise((r) => setTimeout(r, 900));
  await p.eval(grows);
  await new Promise((r) => setTimeout(r, 900));
  check(await p.eval(bottomGap) > 300, `newest: scroll bar held, PageUp, let go -> stays (gap ${await p.eval(bottomGap)})`);
  // End from above the newest rows (scrolled up with the wheel first): following after the hold ends.
  await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: -400 })); window.scrollBy(0, -400)");
  await new Promise((r) => setTimeout(r, 900));
  await p.eval(`document.documentElement.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true })); ${key("End")}; window.scrollTo(0, document.documentElement.scrollHeight)`);
  await new Promise((r) => setTimeout(r, 300));
  await p.eval("document.documentElement.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }))");
  await new Promise((r) => setTimeout(r, 900));
  await p.eval(grows);
  check(await p.until(atEnd, 2000) === true, `newest: scroll bar held, End, let go -> follows (gap ${await p.eval(bottomGap)})`);
  // The page scrolls to its end, and in that frame the reader scrolls up: the page's scroll event never arrives on its
  // own. Later the reader scrolls back to that same end: that is the reader, following again.
  await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: 400 })); window.scrollTo(0, document.documentElement.scrollHeight)");
  await new Promise((r) => setTimeout(r, 900));
  await p.eval(`(() => { const ro = new ResizeObserver(() => { ro.disconnect();
    window.dispatchEvent(new WheelEvent('wheel', { deltaY: -300 })); window.scrollBy(0, -300); });
    ro.observe(document.body); ${grows}; })()`);
  await new Promise((r) => setTimeout(r, 900));
  check(await p.eval(bottomGap) > 200, `newest: scrolled up in the frame the page scrolled itself -> stays (gap ${await p.eval(bottomGap)})`);
  await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: 300 })); window.scrollTo(0, document.documentElement.scrollHeight)");
  await new Promise((r) => setTimeout(r, 900));
  await p.eval(grows);
  check(await p.until(atEnd, 2000) === true, `newest: then scrolled back to that end -> follows again (gap ${await p.eval(bottomGap)})`);
  const up = async () => {   // the reader scrolls up: not following
    await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: -300 })); window.scrollBy(0, -300)");
    await new Promise((r) => setTimeout(r, 900));
  };
  const down = async () => {   // the reader scrolls to the end: following
    await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: 300 })); window.scrollTo(0, document.documentElement.scrollHeight)");
    await new Promise((r) => setTimeout(r, 900));
  };
  // Not following, then the page gets shorter and sits at its end by itself. The reader presses End (nothing moves):
  // following again.
  await p.eval("(() => { const d = document.createElement('div'); d.id = 'shrinks'; d.style.height = '600px'; document.body.append(d); })()");
  await down();
  await up();
  await p.eval("document.getElementById('shrinks').remove()");
  await new Promise((r) => setTimeout(r, 300));
  await p.eval(key("End"));
  await new Promise((r) => setTimeout(r, 900));
  await p.eval(grows);
  check(await p.until(atEnd, 2000) === true, `newest: at the end after the page got shorter, End -> follows (gap ${await p.eval(bottomGap)})`);
  // The reader scrolls to the end and new rows come in before the scrolling settles: following.
  await up();
  await p.eval(`${key("End")}; window.scrollTo(0, document.documentElement.scrollHeight)`);
  await new Promise((r) => setTimeout(r, 50));
  await p.eval(grows);
  await new Promise((r) => setTimeout(r, 900));
  await p.eval(grows);
  check(await p.until(atEnd, 2000) === true, `newest: scrolled to the end, rows came in right after -> follows (gap ${await p.eval(bottomGap)})`);
  // The reader scrolls and opens another agent in the same moment: the other agent opens at its newest message.
  const here = await p.eval("location.hash");
  await p.eval("location.hash = '#pane=w2%3Ap2'");
  await p.until(atEnd, 3000);
  await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: -100 })); window.scrollTo(0, 300)");
  await new Promise((r) => setTimeout(r, 900));
  await p.eval(`window.dispatchEvent(new WheelEvent('wheel', { deltaY: -100 })); window.scrollBy(0, -100); location.hash = ${JSON.stringify(here)}`);
  await new Promise((r) => setTimeout(r, 900));
  check(await p.until(atEnd, 2000) === true, `newest: scrolling, then another agent at once -> it opens at the bottom (gap ${await p.eval(bottomGap)})`);
  // Scrolling while the connection drops: the short "Not connected" screen moving the page is not the reader. Back
  // online, the reader who was scrolled up is not pulled to the bottom.
  await down();
  await up();
  const had = await p.eval(rows);
  await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: -50 })); window.__wheels = setInterval(() => window.dispatchEvent(new WheelEvent('wheel', { deltaY: -50 })), 50); window.__sockets.at(-1).close()");
  await p.until("document.body.innerText.includes('Not connected')");
  await new Promise((r) => setTimeout(r, 100));
  await p.eval("clearInterval(window.__wheels)");
  await p.until(`${rows} >= ${had}`, 8000);
  grow("s3", 1);
  await p.until(`${rows} >= ${had + 1}`);
  await new Promise((r) => setTimeout(r, 500));
  check(await p.eval(bottomGap) > 200, `newest: scrolling when the connection dropped -> not pulled to the bottom after it returns (gap ${await p.eval(bottomGap)})`);
  await down();
  // End is the reader asking for the bottom: new rows that render before the scroll is seen still leave it following.
  await up();
  await p.eval(`${key("End")}; window.scrollTo(0, document.documentElement.scrollHeight); ${grows}`);
  await new Promise((r) => setTimeout(r, 900));
  await p.eval(grows);
  check(await p.until(atEnd, 2000) === true, `newest: End, rows rendered before the scroll -> follows (gap ${await p.eval(bottomGap)})`);
  // From the bottom up in 1 px steps within one round: more than 1 px above the bottom it reached, not following.
  await down();
  await p.eval(`(() => { window.dispatchEvent(new WheelEvent('wheel', { deltaY: 100 })); window.scrollTo(0, document.documentElement.scrollHeight);
    let n = 0; window.__steps = setInterval(() => { window.dispatchEvent(new WheelEvent('wheel', { deltaY: -1 })); window.scrollBy(0, -1);
      if (++n === 70) clearInterval(window.__steps); }, 20); })()`);
  await new Promise((r) => setTimeout(r, 2000));
  await p.eval(grows);
  await new Promise((r) => setTimeout(r, 500));
  check(await p.eval(bottomGap) > 100, `newest: up from the bottom in 1 px steps -> stays (gap ${await p.eval(bottomGap)})`);
  // The page gets shorter during a round and that puts the view at the bottom: not the reader, not following.
  await p.eval("(() => { const d = document.createElement('div'); d.id = 'shrinks2'; d.style.height = '600px'; document.body.append(d); })()");
  await down();
  await up();
  await p.eval(`window.dispatchEvent(new WheelEvent('wheel', { deltaY: -50 })); window.__wheels = setInterval(() => window.dispatchEvent(new WheelEvent('wheel', { deltaY: -50 })), 50);
    document.getElementById('shrinks2').remove()`);
  await new Promise((r) => setTimeout(r, 300));
  await p.eval("clearInterval(window.__wheels)");
  await new Promise((r) => setTimeout(r, 900));
  await p.eval(grows);
  await new Promise((r) => setTimeout(r, 500));
  // (Not pulled to the bottom. The browser's own scroll anchoring may move the view down by part of the new height.)
  check(await p.eval(bottomGap) > 50, `newest: put at the bottom by a shorter page during a round -> not following (gap ${await p.eval(bottomGap)})`);
  // Another session in the same pane (a new agent there): it opens at its newest message, and follows.
  await up();
  const s9 = "document.body.innerText.includes('New session reply 39')";
  hub.stdin.write("session w2:p1 s9\n");
  await p.until(s9, 4000);
  await new Promise((r) => setTimeout(r, 900));
  grow("s9", 2);   // and it keeps following
  await p.until("document.body.innerText.includes('Reply 41.')", 4000);
  await new Promise((r) => setTimeout(r, 500));
  check(await p.until(atEnd, 2000) === true, `newest: another session in the pane -> at its newest message, following (gap ${await p.eval(bottomGap)})`);
  hub.stdin.write("session w2:p1 s3\n");
  await p.until(`!${s9}`, 4000);
  await p.until(atEnd, 3000);
  // The page goes to the background and the browser moves it meanwhile (a restored page, a closed keyboard). That is not
  // the reader either: back in front, the page is at the newest message.
  await p.eval(`Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'hidden' });
    document.dispatchEvent(new Event('visibilitychange')); window.scrollTo(0, 0)`);
  await new Promise((r) => setTimeout(r, 300));
  await p.eval(`delete document.visibilityState; document.dispatchEvent(new Event('visibilitychange'))`);
  check(await p.until(atEnd, 2000) === true, `newest: back from the background -> at the bottom (gap ${await p.eval(bottomGap)})`);

  await pictures();
  await team();
  await twoColumns();

  for (const x of pages) check(x.errors.length === 0, `page: no errors or CSP violations (${x.errors.join(" | ")})`);
}

/** In each page before its own scripts: each View Transition (when it started, the rows and the named nodes right after
 *  the update, when it finished), every frame's length, and `__m.late`: each `conversation` message held that many ms
 *  (the phone's round trip over Tailscale). */
function morphProbe() { return `(() => {
  const T = window.__m = { late: 0, vt: [], frames: [] };
  const svt = Document.prototype.startViewTransition;
  if (svt) Document.prototype.startViewTransition = function (cb) {
    const r = { start: performance.now(), end: 0, rows: -1, named: [], kept: false };
    T.vt.push(r);
    const t = svt.call(this, () => { cb(); r.rows = document.querySelectorAll('.items > [data-id]').length;
      r.named = [...document.querySelectorAll('[style*="view-transition-name"]')]; });
    t.finished.then(() => {}, () => {}).then(() => { r.end = performance.now(); r.kept = r.named.length > 0 && r.named.every((e) => e.isConnected); });
    return t;
  };
  const W = window.WebSocket;
  window.WebSocket = function (...a) {
    const s = new W(...a), add = s.addEventListener.bind(s);
    s.addEventListener = (type, f, o) => add(type, type !== 'message' ? f : (e) => {
      let conv = false; try { conv = JSON.parse(e.data).type === 'conversation'; } catch {}
      const run = () => { const t0 = performance.now(); f(e); if (conv) (T.msgs ??= []).push([t0, performance.now() - t0, e.data.length]); };
      if (conv && T.late) { setTimeout(run, T.late); return; }
      run();
    }, o);
    return s;
  };
  window.WebSocket.prototype = W.prototype;
  Object.assign(window.WebSocket, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 });
  let last = 0;
  const tick = (n) => { if (last) T.frames.push([n, n - last]); last = n; requestAnimationFrame(tick); };
  requestAnimationFrame(tick);
})()`; }

/** Opening and closing an agent on a phone is a morph that runs to its end: the row turns into the name capsule and the
 *  round back button comes in. The conversation's first page comes one round trip after the tap (held 60, 150 or 400
 *  ms here) on a CPU 4x slower: the transition is not cut short (its named nodes stay in the page), it lasts its 420 ms,
 *  and (WEB_MORPH_STRICT=1) from the tap to its end no frame takes 60 ms or more. Before, an answer after the transition's start made the bar anew and Chrome ended
 *  the morph at 170-290 ms, and 100 rows in the update took one frame of 100-150 ms. WEB_MORPH_DEBUG=1: each frame. */
/** RGBA pixels of a PNG from Chrome (8-bit RGB or RGBA, no interlace): { w, h, px }. */
function pngPixels(buf) {
  const w = buf.readUInt32BE(16), h = buf.readUInt32BE(20), type = buf[25], bpp = type === 6 ? 4 : 3;
  const idat = [];
  for (let i = 8; i < buf.length;) { const n = buf.readUInt32BE(i), t = buf.toString("latin1", i + 4, i + 8); if (t === "IDAT") idat.push(buf.subarray(i + 8, i + 8 + n)); i += 12 + n; }
  const raw = inflateSync(Buffer.concat(idat)), stride = w * bpp, px = Buffer.alloc(w * h * 4);
  let prev = Buffer.alloc(stride);
  for (let y = 0; y < h; y++) {
    const f = raw[y * (stride + 1)], line = Buffer.from(raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1)));
    for (let x = 0; x < stride; x++) {
      const a = x >= bpp ? line[x - bpp] : 0, b = prev[x], c = x >= bpp ? prev[x - bpp] : 0;
      const pa = Math.abs(b - c), pb = Math.abs(a - c), pc = Math.abs(a + b - 2 * c);
      line[x] = (line[x] + [0, a, b, (a + b) >> 1, pa <= pb && pa <= pc ? a : pb <= pc ? b : c][f]) & 255;
    }
    for (let x = 0; x < w; x++) for (let k = 0; k < 4; k++) px[(y * w + x) * 4 + k] = k < bpp ? line[x * bpp + k] : 255;
    prev = line;
  }
  return { w, h, px };
}
/** The share of pixels that differ (any channel by more than 24) between two same-size PNGs. */
function pngDiff(a, b) {
  const A = pngPixels(a), B = pngPixels(b);
  if (A.w !== B.w || A.h !== B.h) return 1;
  let n = 0;
  for (let i = 0; i < A.px.length; i += 4) if ([0, 1, 2].some((k) => Math.abs(A.px[i + k] - B.px[i + k]) > 24)) n++;
  return n / (A.w * A.h);
}

/** The composer sits at the bottom from the first frame an agent opens: a short conversation, or one still loading,
 *  does not leave it half-way up the screen until the rows fill it. Where it sits is where a long conversation at its
 *  newest has it: the window's bottom less the page's bottom padding (safe area included). */
/** A note from another agent (`hc send`): `serve` sends its user item with `from` (the sender as hc wrote it). The
 *  fake hub has no notes: three of its user items are made into notes as they come in, as `serve` sends them. The page
 *  shows each as one line on the left, "↳ <name>: <first line>" (the name after the terminal id, else the terminal id,
 *  else "agent"), that opens to the whole note in Markdown. Before, it was a right-aligned bubble, like the owner's own
 *  message. */
async function agentNote() {
  phase("agent note");
  const p = await page(await context(), true);
  pages.push(p);
  await p.onEveryLoad(`(() => {
    const notes = { "pnote-g0": "t2 docs-worker", "pnote-g3": "t3", "pnote-g6": "" };
    const W = window.WebSocket;
    window.WebSocket = function (...a) {
      const s = new W(...a), add = s.addEventListener.bind(s);
      s.addEventListener = (type, f, o) => add(type, type !== 'message' ? f : (e) => {
        let m = null; try { m = JSON.parse(e.data); } catch {}
        if (m?.type !== 'conversation') return f(e);
        for (const it of m.items) if (it.id in notes) { it.from = notes[it.id]; it.text = "Done: notes updated.\\n\\nThe **second** paragraph."; }
        f(new MessageEvent('message', { data: JSON.stringify(m) }));
      }, o);
      return s;
    };
    window.WebSocket.prototype = W.prototype;
    Object.assign(window.WebSocket, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 });
  })()`);
  await p.goto(origin + "/");
  await p.eval("localStorage.setItem('flockbook.token', 'input-token')");
  hub.stdin.write("session w2:p2 pnote 0\ngrow pnote 10\n");
  try {
    await p.eval("location.hash = '#pane=w2%3Ap2'; true");
    await p.reload();
    const at = "document.querySelector(\"section.conversation .items > [data-id='pnote-g9']\")";
    check(!!await p.until(`!!${at}`, webLimit), "note: the conversation with the notes loads");
    const read = (id) => p.eval(`(() => { const e = document.querySelector("section.conversation .items > [data-id='${id}']"); if (!e) return null;
      const items = e.parentElement.getBoundingClientRect(), line = e.querySelector('summary') ?? e, r = line.getBoundingClientRect();
      const body = e.querySelector('.note-body'), shown = !!body && body.checkVisibility();
      return { bubble: !!e.querySelector('.msg.user') || e.matches('.msg.user'), line: line.textContent.trim(), left: Math.round(r.left - items.left),
        right: Math.round(items.right - r.right), oneLine: r.height < 48, shown, strong: shown ? body.querySelector('strong')?.textContent ?? null : null }; })()`);
    const a = await read("pnote-g0");
    check(!!a && !a.bubble && a.line.startsWith("↳ docs-worker: Done: notes updated.") && !a.line.includes("second") && a.oneLine,
      `note: one line, "↳ docs-worker: <first line>", not a bubble (${JSON.stringify(a)})`);
    check(!!a && a.left <= 4, `note: on the left, where the agent's side is (a bubble starts at the right) (${JSON.stringify(a)})`);
    check(!!a && !a.shown, `note: closed at first (${JSON.stringify(a)})`);
    const b = await read("pnote-g3"), c = await read("pnote-g6");
    check(!!b && b.line.startsWith("↳ t3: ") && !!c && c.line.startsWith("↳ agent: "),
      `note: with no name, the terminal id; with neither, "agent" (${b?.line} | ${c?.line})`);
    await p.eval(`document.querySelector("section.conversation .items > [data-id='pnote-g0'] summary")?.click(); true`);
    await twoFrames(p);
    const open = await read("pnote-g0");
    check(!!open && open.shown && open.strong === "second", `note: a tap opens the whole note, in Markdown (${JSON.stringify(open)})`);
    const user = await read("pnote-g9");
    check(!!user && user.bubble, `note: the owner's own message is still a bubble (${JSON.stringify(user)})`);
  } finally {
    hub.stdin.write("session w2:p2 s4\n");
  }
  await p.close();
  pages.splice(pages.indexOf(p), 1);
  check(p.errors.length === 0, `note: no errors (${p.errors.join(" | ")})`);
}

async function composerPin() {
  phase("composer pin");
  const p = await page(await context(), true);
  pages.push(p);
  await p.cdp("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: "no-preference" }] });
  await p.onEveryLoad(morphProbe());
  await p.cdp("Page.bringToFront");
  await p.goto(origin + "/");
  await p.eval("localStorage.setItem('flockbook.token', 'input-token')");
  /** Opens w2:p2 (session `session`, `rows` rows, each `conversation` held `late` ms); every frame from the tap until
   *  the page is settled (the morph ended, the rows the same for 30 frames): how far the composer's bottom is from its
   *  place. */
  const open = async (session, rows, late, rich = 0) => {
    hub.stdin.write(`session w2:p2 ${session} ${rows}\n` + (rich ? `rich ${session} ${rich}\n` : ""));
    await p.eval("history.replaceState(null, '', '/')");
    await p.reload();
    if (!await p.until("!!document.querySelector(\".row[data-pane='w2:p2']\") && !!window.__m", webLimit)) throw new Error("composer: the list did not load");
    await p.until(`document.readyState === 'complete' && ${webSettled}`, webLimit);
    await twoFrames(p);
    await p.eval(`window.__m.late = ${late}; window.__pin = []; window.__pinDone = false; true`);
    await p.eval(`(() => { const t0 = performance.now(), n0 = window.__m.vt.length; const main = document.querySelector('main');
      let same = 0, last = -1;
      const tick = () => { const c = document.querySelector('section.conversation .composer');
        const n = document.querySelectorAll('section.conversation .items > [data-id]').length;
        if (c) { const r = c.getBoundingClientRect(); window.__pin.push([Math.round(performance.now() - t0), Math.round(r.bottom), Math.round(innerHeight - parseFloat(getComputedStyle(main).paddingBottom)), n]); }
        same = c && n === last ? same + 1 : 0; last = n;
        const ended = window.__m.vt.length > n0 && window.__m.vt.every((v) => v.end > 0);
        if ((ended && same >= 30) || performance.now() - t0 > ${webLimit}) { window.__pinDone = true; return; }
        requestAnimationFrame(tick); };
      requestAnimationFrame(tick); document.querySelector(".row[data-pane='w2:p2']").click(); return true; })()`);
    await p.until("window.__pinDone", webLimit + 5000);
    return p.eval(`(() => { const f = window.__pin; const off = f.map(([t, b, want]) => b - want);
      const bad = f.filter(([t, b, want]) => Math.abs(b - want) > 2);
      return { frames: f.length, worst: Math.max(0, ...off.map(Math.abs)), first: f[0] ?? null, last: f.at(-1) ?? null, bad: bad.slice(0, 3) }; })()`);
  };
  const ok = (r) => !!r && r.frames >= 10 && r.worst <= 2;
  const says = (r) => JSON.stringify(r);
  await p.cdp("Emulation.setCPUThrottlingRate", { rate: webCPU });
  try {
    const long = await open("pinlong", 0, 0, 60);
    check(!!long?.last && Math.abs(long.last[1] - long.last[2]) <= 2, `composer: a long conversation at its newest, the composer at the bottom (the place the others are held to) (${says(long?.last)})`);
    const two = await open("pin2", 2, 0);
    check(ok(two), `composer: two messages -> at the bottom from the first frame to the end (${says(two)})`);
    const one = await open("pin1", 1, 0);
    check(ok(one), `composer: one message -> at the bottom from the first frame to the end (${says(one)})`);
    const late = await open("pinlate", 40, 400);
    check(ok(late), `composer: rows 400 ms late (loading, then filled) -> at the bottom all along (${says(late)})`);
  } finally {
    await p.cdp("Emulation.setCPUThrottlingRate", { rate: 1 });
    hub.stdin.write("session w2:p2 s4\n");
  }
  await p.close();
  pages.splice(pages.indexOf(p), 1);
  check(p.errors.length === 0, `composer: no errors (${p.errors.join(" | ")})`);
}

async function morph() {
  phase("morph");
  const p = await page(await context(), true);
  pages.push(p);
  // A CI runner's macOS may have Reduce Motion on: the page then changes screens with no morph, as it should.
  await p.cdp("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: "no-preference" }] });
  await p.onEveryLoad(morphProbe());
  await p.cdp("Page.bringToFront");
  await p.goto(origin + "/");
  await p.eval("localStorage.setItem('flockbook.token', 'input-token')");
  hub.stdin.write("session w2:p2 morph 0\nrich morph 100\n");
  // Waits on states (see webLimit): the CPU slowed webCPU times.
  const strict = webStrict, CPU = webCPU, LIMIT = webLimit, settled = webSettled;
  const frames = () => twoFrames(p);
  const listed = async () => {
    await p.eval("history.replaceState(null, '', '/')");
    await p.reload();
    if (!await p.until("!!document.querySelector(\".row[data-pane='w2:p2']\") && !!window.__m", LIMIT)) throw new Error("morph: the list did not load");
    await p.until(`document.readyState === 'complete' && ${settled}`, LIMIT);
    await frames();
  };
  await listed();
  const rows = "document.querySelectorAll('section.conversation .items > [data-id]').length";
  const atEnd = "Math.abs(document.documentElement.scrollHeight - window.innerHeight - window.scrollY) <= 4";
  /** Taps the row (or Back) with each `conversation` held `late` ms; the transition's numbers once it ended. */
  const go = async (late, open) => {
    await p.eval(`window.__m.late = ${late}`);
    const n = await p.eval("window.__m.vt.length");
    const tap = await p.eval(`(() => { const e = document.querySelector(${JSON.stringify(open ? ".row[data-pane='w2:p2']" : "section.conversation .top .back")});
      if (!e) return 0; const t = performance.now(); e.click(); return t; })()`);
    if (!tap) return null;
    // Not ended within the limit: why, as the result (it has no rows: the checks print it whole).
    if (!await p.until(`window.__m.vt[${n}]?.end > 0`, LIMIT)) {
      return p.eval(`({ started: window.__m.vt.length > ${n}, visible: document.visibilityState, transitions: !!document.startViewTransition,
        still: matchMedia('(prefers-reduced-motion: reduce)').matches, wide: innerWidth })`);
    }
    // The frames up to 20 ms after the end are counted: wait for one drawn after that.
    await p.until(`window.__m.frames.some(([t]) => t > window.__m.vt[${n}].end + 20)`, LIMIT);
    return p.eval(`(() => { const r = window.__m.vt[${n}];
      const f = window.__m.frames.filter(([t]) => t > ${tap} && t <= r.end + 20).map(([, d]) => d);
      return { wait: Math.round(r.start - ${tap}), ms: Math.round(r.end - r.start), rows: r.rows, kept: r.kept, named: r.named.length, worst: Math.round(Math.max(0, ...f)), f: ${!!process.env.WEB_MORPH_DEBUG} ? window.__m.frames.filter(([t]) => t > ${tap} - 50 && t <= r.end + 20).map(([t, d]) => Math.round(t - ${tap}) + ":" + Math.round(d)).join(" ") : undefined, msgs: ${!!process.env.WEB_MORPH_DEBUG} ? (window.__m.msgs ?? []).filter(([t]) => t > ${tap} - 50).map(([t, d, n]) => Math.round(t - ${tap}) + ":" + Math.round(d) + "ms/" + n).join(" ") : undefined }; })()`);
  };
  const says = (r) => JSON.stringify(r);
  const ran = (r) => !!r && r.ms >= 400 && r.kept;
  // At most two frames missed in a row (50 ms) while the page is loaded; 60 sits above that.
  const fast = (r) => !!r && r.worst < 60;
  // Frame times and waits are checked only with WEB_MORPH_STRICT=1 (a quiet machine): on a shared CI runner they vary
  // (100-333 ms frames seen), so there they are printed, and the morph running to its end is what fails.
  const timing = (ok, name) => { if (strict) check(ok, name); else console.log(`info ${name}: ${ok ? "within" : "over"} the strict budget`); };
  await p.cdp("Emulation.setCPUThrottlingRate", { rate: CPU });
  try {
    // First open after the page loads, the answer before the wait's end (60 ms): the transition starts with the rows
    // (about a screen of them). The page's first conversation costs one longer frame once (styles, fonts: 49-51 ms
    // here; an open after another agent was opened once, 18-33 ms); 100 rows in the update took 101-152 ms.
    let a = await go(60, true);
    // That one frame varies with the machine (twice in 30 runs here it was 99-134 ms): a second page load once.
    if (strict && a && a.worst >= 80) { await listed(); a = await go(60, true); }
    check(ran(a), `morph: first open, answer 60 ms late (CPU ${CPU}x) -> the morph runs to its end, its nodes kept (${says(a)})`);
    check(!!a && a.rows >= 1 && a.rows <= 15, `morph: the first page is in the new picture, about a screen of rows only (${a?.rows ?? says(a)})`);
    timing(!!a && a.worst < 80, `morph: the page's first open, no frame of 80 ms or more (worst ${a?.worst})`);
    check(await p.until(`${rows} >= 100`, LIMIT) === true && await p.until(atEnd, LIMIT) === true,
      `morph: after the morph the other rows go in, the page at the newest (${await p.eval(rows)})`);
    // The bar the morph ends in: a round back button with the arrow only, and the name capsule beside it.
    const bar = await p.eval(`(() => { const b = document.querySelector('section.conversation > .top > .back'), c = document.querySelector('section.conversation > .top > .bar-title');
      if (!b || !c) return null; const rb = b.getBoundingClientRect(), rc = c.getBoundingClientRect(), sb = getComputedStyle(b), sc = getComputedStyle(c);
      return { label: b.getAttribute('aria-label'), text: b.textContent.trim(), svg: !!b.querySelector('svg'), bw: rb.width, bh: rb.height, round: parseFloat(sb.borderRadius) >= rb.height / 2 - 1,
        gap: rc.left - rb.right, ch: rc.height, title: c.textContent, glass: sb.backgroundColor !== 'rgba(0, 0, 0, 0)' && sc.backgroundColor !== 'rgba(0, 0, 0, 0)',
        bar: getComputedStyle(b.parentElement).backgroundColor }; })()`);
    check(!!bar && bar.label === "Agents" && bar.text === "" && bar.svg && bar.bw >= 44 && bar.bh >= 44 && bar.round,
      `morph: the back button is round, the arrow only, 44 pt at least, named Agents (${says(bar)})`);
    check(!!bar && bar.gap >= 4 && bar.ch >= 44 && bar.title.includes("Notes") && bar.glass && bar.bar === "rgba(0, 0, 0, 0)",
      `morph: beside it a capsule of its own with the glyph and the name, both glass, no bar behind them (${says(bar)})`);
    const b1 = await go(150, false);
    check(ran(b1), `morph: close (CPU ${CPU}x) -> runs to its end (${says(b1)})`);
    timing(fast(b1), `morph: close, no frame of 60 ms or more (worst ${b1?.worst})`);
    // Again: rows known, the answer 150 ms late comes during the morph and does not cut it.
    const c = await go(150, true);
    check(ran(c), `morph: second open, answer 150 ms late -> runs to its end, not cut by the answer (${says(c)})`);
    timing(fast(c) && c.wait < 60, `morph: second open, starts at once, no frame of 60 ms or more (wait ${c?.wait}, worst ${c?.worst})`);
    const d = await go(150, false);
    check(ran(d), `morph: second close (${says(d)})`);
    timing(fast(d), `morph: second close, no frame of 60 ms or more (worst ${d?.worst})`);
    // A new page, another agent opened once: then this one's first open (its rows built now) has no long frame.
    await listed();
    await p.cdp("Emulation.setCPUThrottlingRate", { rate: 1 });
    // Each change of screen waits for its morph's end: a tap during one changes the screen with no morph (by design),
    // and on a slow runner the close took over 600 ms.
    await p.eval("location.hash = '#pane=w1%3Ap1'");
    await p.until("!!document.querySelector('section.conversation .items > [data-id]')", LIMIT);
    await p.until(settled, LIMIT);
    await frames();
    await p.eval("location.hash = ''");
    await p.until("!!document.querySelector(\".row[data-pane='w2:p2']\")", LIMIT);
    await p.until(settled, LIMIT);
    await frames();
    await p.cdp("Emulation.setCPUThrottlingRate", { rate: CPU });
    const w = await go(60, true);
    check(ran(w) && w.rows >= 1 && w.rows <= 15, `morph: first open of an agent after another, answer 60 ms late -> runs to its end, a screen of rows (${says(w)})`);
    timing(fast(w), `morph: first open of an agent after another, no frame of 60 ms or more (worst ${w?.worst})`);
    await go(0, false);
    // First open again, the answer after the wait (400 ms late): the morph starts after at most 150 ms, from the empty
    // sheet, and the rows coming in during it leave its nodes in place.
    await listed();
    const e = await go(400, true);
    check(ran(e), `morph: answer 400 ms late -> not cut when the rows come during the morph (${says(e)})`);
    timing(fast(e) && e.wait < 250, `morph: answer 400 ms late, starts after the 150 ms wait, no frame of 60 ms or more (wait ${e?.wait}, worst ${e?.worst})`);
    check(await p.until(`${rows} >= 100`, LIMIT) === true, `morph: and then the rows are all in (${await p.eval(rows)})`);
    // Mid-morph the rows (fb-body) stacked above the capsule, and its glass (no backdrop blur in a snapshot) showed
    // them through: the name was hidden until the end. Paused at 300 ms: the bar's pieces above the rows, a solid
    // backing under the capsule; once ended, the name is what is on top at its place.
    await go(0, false);
    await p.cdp("Emulation.setCPUThrottlingRate", { rate: 1 });
    await p.eval(`(() => { const svt = document.startViewTransition; document.startViewTransition = function (cb) {
      document.startViewTransition = svt; const t = svt.call(this, cb);
      t.ready.then(() => { for (const a of document.getAnimations().filter((a) => a.effect?.pseudoElement)) { a.pause(); a.currentTime = 300; } window.__paused = true; });
      return t; }; window.__paused = false; return true; })()`);
    await p.eval(`document.querySelector(".row[data-pane='w2:p2']").click()`);
    await p.until("window.__paused", LIMIT);
    const stack = await p.eval(`(() => { const css = (pe) => getComputedStyle(document.documentElement, pe);
      const z = (n) => parseInt(css('::view-transition-group(' + n + ')').zIndex) || 0;
      const bg = css('::view-transition-new(fb-sheet)').backgroundColor, alpha = /rgba/.test(bg) ? parseFloat(bg.split(',')[3]) : bg.startsWith('rgb') ? 1 : 0;
      return { body: z('fb-body'), sheet: z('fb-sheet'), dot: z('fb-dot'), title: z('fb-title'), back: z('fb-back'), bg, alpha }; })()`);
    await p.eval("document.getAnimations().filter((a) => a.effect?.pseudoElement).forEach((a) => a.finish()); true");
    await p.until(settled, LIMIT);
    await frames();
    const seen = await p.eval(`(() => { const n = document.querySelector('section.conversation > .top .bar-name'); if (!n) return null;
      const r = n.getBoundingClientRect(), at = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      return { name: n.textContent, on: !!at && n.contains(at), w: Math.round(r.width) }; })()`);
    check(!!stack && Math.min(stack.sheet, stack.dot, stack.title, stack.back) > stack.body && stack.alpha === 1,
      `morph: mid-morph the capsule and the back button are above the rows, the capsule on a solid backing (${says(stack)})`);
    check(!!seen && seen.on && seen.w > 0 && seen.name.includes("Notes"), `morph: after the morph the name is on top in its capsule (${says(seen)})`);
    // Back to the list, paused at 300 ms. The back button's glass (backdrop-filter) stayed on its group for the whole
    // morph, a blurred square over the list's title; and the tap scrolled the conversation to its top (the empty
    // fragment) before the old picture was taken, so the close started from the wrong place.
    await p.until(`${rows} >= 100`, LIMIT);
    await p.eval("window.scrollTo(0, document.documentElement.scrollHeight - innerHeight - 300); true");
    await frames();
    const was = await p.eval("Math.round(scrollY)");
    await p.eval(`(() => { const svt = document.startViewTransition; window.__close = { start: -1, paused: false }; document.startViewTransition = function (cb) {
      document.startViewTransition = svt; window.__close.start = Math.round(scrollY); const t = svt.call(this, cb);
      t.ready.then(() => { for (const a of document.getAnimations().filter((a) => a.effect?.pseudoElement)) { a.pause(); a.currentTime = 300; } window.__close.paused = true; });
      return t; }; return true; })()`);
    await p.eval("document.querySelector('section.conversation .top .back').click(); true");
    await p.until("window.__close.paused", LIMIT);
    await frames();
    const clip = await p.eval(`(() => { const t = document.querySelector('header h1, .top h1') ?? document.querySelector('h1'); const r = t?.getBoundingClientRect();
      return r ? { x: 0, y: 0, width: Math.ceil(r.right + 8), height: Math.ceil(r.bottom + 8), scale: 1 } : null; })()`);
    const shoot = async () => clip ? Buffer.from((await p.cdp("Page.captureScreenshot", { format: "png", clip })).data, "base64") : null;
    const mid = await shoot();
    const start = await p.eval("window.__close.start");
    await p.eval("document.getAnimations().filter((a) => a.effect?.pseudoElement).forEach((a) => a.finish()); true");
    await p.until(settled, LIMIT);
    await frames();
    const end = await shoot();
    const diff = mid && end ? Math.round(pngDiff(mid, end) * 1000) / 10 : 100;
    check(!!clip && diff < 2, `morph: back to the list, at 300 ms nothing covers the list's title (pixels differing from the end: ${diff}%, ${says(clip)})`);
    check(start === was, `morph: back to the list starts from where the conversation was read (scrollY ${was} at the tap, ${start} at the start)`);
    // The back button steps back in history (no entry of its own): the phone's Back after it does not open the agent
    // again. (The entry before may be another page load: the page is read again once it settles.)
    const listAt = await p.eval("location.hash");
    await p.eval("window.__moved = false; addEventListener('popstate', () => { window.__moved = true; }, { once: true }); history.back(); true");
    // Settled: the history moved (popstate, or another document: no __moved), the page complete and two frames drawn.
    await p.until("window.__moved !== false", LIMIT);
    await p.until(`document.readyState === 'complete' && !!window.__m && ${settled}`, LIMIT);
    await frames().catch(() => {});
    const after = await p.eval("location.hash + ' ' + !!document.querySelector(\"section.conversation[data-pane='w2:p2']\")").catch((e) => String(e));
    check(listAt === "" && !/w2(%3A|:)p2|true$/.test(after), `morph: after Back to the list, the phone's Back does not open the agent again (list at '${listAt}', then '${after}')`);
  } finally {
    await p.cdp("Emulation.setCPUThrottlingRate", { rate: 1 });
    hub.stdin.write("session w2:p2 s4\n");   // the pane's own session again, for the other sections
  }
  await p.close();
  pages.splice(pages.indexOf(p), 1);
  check(p.errors.length === 0, `morph: no errors or CSP violations (${p.errors.join(" | ")})`);
}

/** iPad and other wide screens (768 pt and up): the agents on the left (320 pt, can be hidden), the conversation on the
 *  right under its own bar, its text and composer at most 760 pt wide in the middle. Narrower (a phone, Split View at a
 *  third): the phone's one column. */
async function twoColumns() {
  phase("wide screens");
  const p = await page(await context());
  pages.push(p);
  const dpr = shots ? 2 : 1;   // not 3: see `scale` and the stills of pictures
  await p.scale(dpr, 1024, 768);
  await p.goto(origin + "/");
  await p.eval("localStorage.setItem('flockbook.token', 'input-token')");
  await p.reload();
  check(await p.until("!!document.querySelector('.list')", 8000) === true, "wide: list shown after hello");
  const box = (sel) => `(e => e && (r => ({ l: Math.round(r.left), r: Math.round(r.right), t: Math.round(r.top), b: Math.round(r.bottom), w: Math.round(r.width) }))(e.getBoundingClientRect()))(document.querySelector(${JSON.stringify(sel)}))`;
  const side = await p.eval(box(".side"));
  check(!!side && side.l === 0 && side.w >= 300 && side.w <= 340, `wide 1024: the agents in a left column of about 320 pt (${JSON.stringify(side)})`);
  check(await p.eval("!!document.querySelector('.side .list') && !document.querySelector('section.conversation')") === true, "wide 1024: no agent chosen -> the list on the left, no conversation");
  check(await p.eval(noScroll), "wide 1024: no sideways scroll");

  // Choosing an agent: the same page and the same list; only the right column changes.
  await p.eval("window.__samePage = true; window.__side = document.querySelector('.side')");
  await p.click(".side [data-pane='w2:p1']");
  check(await p.until("!!document.querySelector('section.conversation .msg')", 8000) === true, "wide: choosing an agent shows its conversation");
  check(await p.eval("window.__samePage === true && document.querySelector('.side') === window.__side && !!window.__side?.isConnected && getComputedStyle(window.__side).display !== 'none'") === true,
    "wide: choosing an agent keeps the page and the list (the right column changes)");
  check(await p.eval("document.querySelector(\".side [data-pane='w2:p1']\")?.getAttribute('aria-current')") === "page", "wide: the chosen agent is marked in the list");
  check(await p.eval("document.querySelector('.side')?.getAttribute('aria-live')") === "off", "wide: the list is not read out again on every tick");
  check(await p.eval("!!document.querySelector('.side')?.id && document.querySelector('#side-toggle')?.getAttribute('aria-controls') === document.querySelector('.side').id") === true,
    "wide: the list button names the list it hides");
  // Another agent: the list stays in the page, nothing writes its scroll on each tick, and the row pressed keeps the focus.
  await p.eval(`window.__sideOut = 0; window.__sideWrites = 0;
    new MutationObserver((ms) => { for (const m of ms) for (const n of m.removedNodes) if (n === window.__side) window.__sideOut++; }).observe(document.getElementById('app'), { childList: true });
    { const d = Object.getOwnPropertyDescriptor(Element.prototype, 'scrollTop');
      Object.defineProperty(window.__side, 'scrollTop', { get() { return d.get.call(this); }, set(v) { window.__sideWrites++; d.set.call(this, v); }, configurable: true }); }
    { const r = document.querySelector(".side [data-pane='w1:p2']"); r?.focus(); r?.click(); }`);
  await p.until("document.querySelector('section.conversation')?.dataset.pane === 'w1:p2'", 8000);
  await new Promise((r) => setTimeout(r, 1500));
  check(await p.eval("window.__sideOut === 0 && document.querySelector('.side') === window.__side") === true, `wide: another agent keeps the list in the page (taken out ${await p.eval("window.__sideOut")}x)`);
  check(await p.eval("window.__sideWrites") === 0, `wide: no scroll written to the list on each tick (${await p.eval("window.__sideWrites")})`);
  check(await p.eval("document.activeElement?.dataset?.pane") === "w1:p2", `wide: the row pressed keeps the focus (${await p.eval("document.activeElement?.outerHTML.slice(0, 80)")})`);
  // Back: the agent before, beside the same list.
  await p.eval("history.back()");
  check(await p.until("document.querySelector('section.conversation')?.dataset.pane === 'w2:p1' && document.querySelector('.side') === window.__side", 8000) === true,
    "wide: Back -> the agent before, beside the same list");
  const geo = async (w) => {
    const top = await p.eval(box("section.conversation > .top")), items = await p.eval(box("section.conversation > .items"));
    const form = await p.eval(box("section.conversation > form.composer")), s = await p.eval(box(".side"));
    const left = s ? s.r : 0;
    check(!!top && Math.abs(top.l - left) <= 1 && Math.abs(top.r - w) <= 1, `wide ${w}: the bar spans the right column (${left}..${w}: ${JSON.stringify(top)})`);
    check(!!items && items.w <= 760 && items.l >= left && Math.abs((items.l - left) - (w - items.r)) <= 2,
      `wide ${w}: the text at most 760 pt, in the middle of the right column (${JSON.stringify(items)})`);
    check(!!form && Math.abs(form.l - items.l) <= 1 && Math.abs(form.r - items.r) <= 1, `wide ${w}: the composer as wide as the text (${JSON.stringify(form)})`);
  };
  await geo(1024);
  // The bar is opaque enough that the text under it does not show through.
  const alpha = `(c => { const m = /rgba?\\(([^)]*)\\)/.exec(c.backgroundColor); const v = m ? m[1].split(',') : []; return v.length === 4 ? Number(v[3]) : v.length ? 1 : 0; })(getComputedStyle(document.querySelector('section.conversation > .top') ?? document.createElement('i')))`;
  const a = await p.eval(alpha);
  check(a >= 0.9, `wide: the bar is opaque enough (alpha ${a})`);

  // The list can be hidden and shown again; hidden, the conversation takes the whole width.
  check(await p.click("#side-toggle") === true, "wide: a button hides the list");
  check(await p.until("!document.querySelector('.side') || getComputedStyle(document.querySelector('.side')).display === 'none'") === true, "wide: the list hidden");
  await geo(1024);
  check(await p.click("#side-toggle") === true && await p.until("document.querySelector('.side') && getComputedStyle(document.querySelector('.side')).display !== 'none'") === true,
    "wide: the button shows the list again");
  // The row being read stays in place when the list is hidden or shown and across 768 (rotation, Split View).
  const mark = `(() => { for (const e of document.querySelectorAll('.items > [data-id]')) { const r = e.getBoundingClientRect(); if (r.bottom > 0) return JSON.stringify({ id: e.dataset.id, top: Math.round(r.top) }); } return 'null'; })()`;
  const stays = async (at, what) => {
    await new Promise((r) => setTimeout(r, 400));
    const now = await p.eval(`Math.round([...document.querySelectorAll('.items > [data-id]')].find((e) => e.dataset.id === ${JSON.stringify(at?.id ?? "")})?.getBoundingClientRect().top ?? -9999)`);
    check(!!at && Math.abs(now - at.top) <= 2, `${what}: the row being read stays in place (${JSON.stringify(at)} -> ${now})`);
    return JSON.parse(await p.eval(mark));
  };
  // Safari has no scroll anchoring: without it here, Chrome would keep the row in place by itself.
  await p.eval("document.documentElement.style.overflowAnchor = document.body.style.overflowAnchor = 'none'");
  await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: -400 })); window.scrollTo(0, Math.round(document.documentElement.scrollHeight / 2))");
  await new Promise((r) => setTimeout(r, 400));
  let at = JSON.parse(await p.eval(mark));
  await p.click("#side-toggle");
  at = await stays(at, "wide: hiding the list");
  await p.click("#side-toggle");
  at = await stays(at, "wide: showing the list");
  await p.scale(dpr, 767, 1024);
  await p.until("!document.querySelector('.side')");
  at = await stays(at, "wide -> narrow");
  await p.scale(dpr, 1024, 768);
  await p.until("!!document.querySelector('.side')");
  at = await stays(at, "narrow -> wide");
  // Chrome keeps the view in place when the window turns or resizes; Safari does not: the rows are laid out again
  // before the page hears of it. Lay them out narrower first, then tell the page.
  await p.eval("document.getElementById('app').style.maxWidth = '520px'; document.body.offsetHeight; window.dispatchEvent(new Event('resize'))");
  await stays(at, "a turn (rows laid out again before the page hears of it)");
  await p.eval("document.getElementById('app').style.maxWidth = ''; window.dispatchEvent(new Event('resize'))");

  // A draft, its box and the focus stay across 768.
  // (w1:p2: earlier checks leave a prompt open on w2:p2, which turns its composer off.)
  await p.eval("location.hash = '#pane=w1%3Ap2'");
  await p.until("document.querySelector('#composer-text')?.disabled === false && document.querySelector('section.conversation')?.dataset.pane === 'w1:p2'", 8000);
  await p.type("#composer-text", "Half a thought.");
  await p.eval("window.__box = document.getElementById('composer-text'); window.__box.focus()");
  for (const [w, ht] of [[767, 1024], [1024, 768]]) {
    await p.scale(dpr, w, ht);
    await p.until(w < 768 ? "!document.querySelector('.side')" : "!!document.querySelector('.side')");
    check(await p.eval("document.getElementById('composer-text') === window.__box && window.__box.value === 'Half a thought.' && document.activeElement === window.__box") === true,
      `${w}: the draft, its box and the focus stay across 768 (${await p.eval("JSON.stringify([document.getElementById('composer-text') === window.__box, document.getElementById('composer-text')?.value, document.activeElement?.id, document.getElementById('composer-text')?.disabled])")})`);
  }
  await p.type("#composer-text", "");
  await p.eval("document.activeElement?.blur(); location.hash = '#pane=w2%3Ap1'");
  await p.until("document.querySelector('section.conversation')?.dataset.pane === 'w2:p1' && !!document.querySelector('section.conversation .msg')", 8000);
  if (shots) {
    await p.eval("window.scrollTo(0, document.documentElement.scrollHeight)");
    await p.shot("ipad-landscape");
    await p.shot("ipad-landscape", true);
  }

  // Portrait (768 wide): still two columns.
  await p.scale(dpr, 768, 1024);
  check(await p.until("!!document.querySelector('.side') && getComputedStyle(document.querySelector('.side')).display !== 'none'") === true, "wide 768: two columns");
  await geo(768);
  check(await p.eval(noScroll), "wide 768: no sideways scroll");
  if (shots) {
    await p.eval("window.scrollTo(0, document.documentElement.scrollHeight)");
    await p.shot("ipad-portrait");
    await p.shot("ipad-portrait", true);
    await p.eval("location.hash = ''");
    await p.until("!document.querySelector('section.conversation')");
    await p.shot("ipad-portrait-list");
    await p.shot("ipad-portrait-list", true);
    await p.eval("location.hash = '#pane=w2%3Ap1'");
    await p.until("!!document.querySelector('section.conversation .msg')", 8000);
  }

  // Under 768 (Split View at a third, a phone) and a phone on its side (wide but low): the phone's one column, as before.
  for (const [w, ht] of [[932, 430], [767, 1024], [375, 1024]]) {
    await p.scale(dpr, w, ht);
    check(await p.until("!document.querySelector('.side') && !!document.querySelector('section.conversation')") === true, `narrow ${w}x${ht}: one column, the conversation only`);
    const main = await p.eval(box("main"));
    check(main.w === Math.min(w, 760), `narrow ${w}: the phone's column (${main.w})`);
    check(await p.eval("document.querySelector('.back')?.getAttribute('aria-label')") === "Agents", `narrow ${w}: Back to the agents`);
    check(await p.eval(noScroll), `narrow ${w}: no sideways scroll`);
  }
  await p.eval("location.hash = ''");
  check(await p.until("!!document.querySelector('.list') && !document.querySelector('section.conversation') && !document.querySelector('.side')") === true, "narrow: Back -> the list fills the page");
  await p.click("[data-pane='w2:p1']");
  check(await p.until("!!document.querySelector('section.conversation') && !document.querySelector('.list')") === true, "narrow: choosing an agent replaces the list");
  // A link to an agent opens it beside the list.
  await p.scale(dpr, 1024, 768);
  await p.eval("location.hash = '#pane=w1%3Ap2'");
  await p.reload();
  check(await p.until("document.querySelector('section.conversation')?.dataset.pane === 'w1:p2' && !!document.querySelector('.side .list')", 8000) === true,
    "wide: a link to an agent opens it beside the list");
  check(await p.eval("document.querySelector(\".side [data-pane='w1:p2']\")?.getAttribute('aria-current')") === "page", "wide: the linked agent is marked in the list");
  await p.close();
  pages.splice(pages.indexOf(p), 1);
  check(p.errors.length === 0, `wide: no errors or CSP violations (${p.errors.join(" | ")})`);
}

/** Pictures (remote-protocol.md 3.4): each keeps its place, is asked for only near the view, stays in memory only, and
 *  the full picture comes on a tap. */
async function pictures() {
  phase("pictures");
  const p = await page(await context());
  pages.push(p);
  await p.onEveryLoad(`{ window.__pics = []; const send = WebSocket.prototype.send;
    WebSocket.prototype.send = function (d) { try { const m = JSON.parse(d); if (m.type === "picture") window.__pics.push(m); } catch {} return send.call(this, d); };
    const make = URL.createObjectURL, revoke = URL.revokeObjectURL; window.__urls = []; window.__revoked = [];
    URL.createObjectURL = (b) => { const u = make(b); window.__urls.push(u); return u; };
    URL.revokeObjectURL = (u) => { window.__revoked.push(u); revoke(u); };
    const IO = IntersectionObserver; window.IntersectionObserver = class extends IO { constructor(cb, o) { super(cb, o); this.__cb = cb; } };
    window.__observed = new Set(); const watch = IO.prototype.observe, unwatch = IO.prototype.unobserve;
    IO.prototype.observe = function (t) { if (t.matches?.("button.picture")) { window.__observed.add(t); window.__pictureIO = this; } return watch.call(this, t); };
    IntersectionObserver.prototype.unobserve = function (t) { window.__observed.delete(t); return unwatch.call(this, t); }; }`);
  hub.stdin.write("pictures w2:p2 s20\n");
  await p.goto(origin + "/");
  await p.eval("localStorage.setItem('flockbook.token', 'input-token')");
  await p.reload();
  await p.until("!!document.querySelector('.list')", 8000);
  await p.eval("location.hash = '#pane=w2%3Ap2'");
  const boxes = "document.querySelectorAll('button.picture')";
  const lastLine = shots ? "Done: the card fits" : "Line 40.";   // the drawn pictures' words (stills)
  check(await p.until(`${boxes}.length === 3 && document.body.innerText.includes(${JSON.stringify(lastLine)})`, 8000) === true,
    `pictures: three picture boxes in the conversation (${await p.eval(`${boxes}.length`)}: ${await p.eval("document.body.innerText.slice(0, 160)")})`);
  // Each box has the picture's shape before anything loads (120x80, 1600x1200 in the bubble; 900x600 under the tool group).
  const shapes = await p.eval(`[...${boxes}].map((b) => { const r = b.getBoundingClientRect(); return [Math.round(r.width), Math.round(r.height)]; })`);
  const ratio = ([w, h], want) => h > 0 && Math.abs(w / h - want) < 0.03;
  check(ratio(shapes[0], 1.5) && shapes[0][0] === 120 && ratio(shapes[1], 4 / 3) && shapes[1][0] > 200 && ratio(shapes[2], 1.5) && shapes[2][0] > 300,
    `pictures: each box keeps the picture's place (${JSON.stringify(shapes)})`);
  check(await p.eval(noScroll), "pictures: no sideways scroll");
  await new Promise((r) => setTimeout(r, 500));
  check(await p.eval("window.__pics.length") === 0, `pictures: none asked for while far above the view (${await p.eval("window.__pics.length")})`);
  // Scrolled into view: each is asked for once as a preview and shows from memory.
  await p.eval(`window.dispatchEvent(new WheelEvent('wheel', { deltaY: -100 })); ${boxes}[0].scrollIntoView({ block: 'start' })`);
  const shown = "[...document.querySelectorAll('button.picture img')].filter((i) => i.src.startsWith('blob:') && i.complete && i.naturalWidth > 0).length";
  check(await p.until(`${shown} === 3`, 8000) === true, `pictures: shown when scrolled into view (${await p.eval(shown)})`);
  const asked = await p.eval("window.__pics");
  check(asked.length === 3 && asked.every((m) => m.full === false && m.pane === "w2:p2"), `pictures: three preview requests (${JSON.stringify(asked)})`);
  check(await p.eval("document.querySelectorAll('button.picture img')[1].naturalWidth") === 1024, "pictures: the preview is at most 1024 px");
  check(await p.eval("caches.keys().then((k) => k.length)") === 0 && await p.eval("Object.keys(localStorage).join()") === "flockbook.token",
    "pictures: nothing in a cache or in storage");
  // Stills at pixel ratio 2: at 3 the headless shell's screenshot never returns with both large pictures in view.
  if (shots) await p.scale(2);
  await p.shot("pictures");
  await p.shot("pictures", true);
  // A tap opens the full picture over the page; closing lets it go.
  await p.click("button.picture[data-index='1']");
  check(await p.until("document.querySelector('.viewer img')?.naturalWidth === 1600", 8000) === true, "pictures: a tap opens the full picture");
  check(await p.eval("window.__pics.some((m) => m.full === true && m.index === 1)") === true, "pictures: the full picture is asked for on the tap only");
  check(await p.eval(noScroll), "pictures: viewer, no sideways scroll");
  await p.shot("picture-viewer");
  await p.shot("picture-viewer", true);
  if (shots) await p.scale(3);
  const full = await p.eval("document.querySelector('.viewer img').src");
  await p.click(".viewer-close");
  check(await p.eval(`!document.querySelector('.viewer') && window.__revoked.includes(${JSON.stringify(full)})`) === true, "pictures: closing the viewer lets the full picture go");
  // Dragged up or down like the Photos app: the picture follows the finger and the page under it shows through.
  // Let go past a quarter of the screen, or flicked, it closes; a short drag springs back. Sideways, two fingers or
  // zoomed in, nothing closes. Each point is [x, y]; one finger unless a second list is given.
  const opened = async () => {
    await p.click("button.picture[data-index='1']");
    return await p.until("document.querySelector('.viewer img')?.naturalWidth === 1600", 8000) === true;
  };
  // Each touch carries its own time (`after` ms past the one before), so the page sees the intended speed however
  // slow the browser is to take them.
  let clock;
  const touch = async (type, touchPoints, after) => {
    clock = type === "touchStart" ? Date.now() / 1000 : clock + after / 1000;
    await p.cdp("Input.dispatchTouchEvent", { type, touchPoints, timestamp: clock });
  };
  const swipe = async (path, { ms = 30, lift = true, second } = {}) => {
    const at = (i) => [{ id: 1, x: path[i][0], y: path[i][1] }, ...(second ? [{ id: 2, x: second[i][0], y: second[i][1] }] : [])];
    await touch("touchStart", at(0), 0);
    for (let i = 1; i < path.length; i++) await touch("touchMove", at(i), ms);
    if (lift) await touch("touchEnd", [], 120);
  };
  const line = (x0, y0, x1, y1, n) => Array.from({ length: n + 1 }, (_, i) => [x0 + (x1 - x0) * i / n, y0 + (y1 - y0) * i / n]);
  const shift = "(() => { const m = getComputedStyle(document.querySelector('.viewer img')).transform; return m === 'none' ? 0 : Math.round(new DOMMatrix(m).m42); })()";
  const fade = "(document.querySelector('.viewer') ? Number(getComputedStyle(document.querySelector('.viewer')).backgroundColor.match(/\\(([^)]*)\\)/)?.[1].match(/[\\d.]+/g)?.[3] ?? 1) : 1)";
  const viewerGone = "!document.querySelector('.viewer')";
  check(await opened(), "pictures: the viewer opens for a drag");
  const dragged = await p.eval("document.querySelector('.viewer img').src");
  await swipe(line(195, 300, 195, 420, 12), { lift: false });
  const midShift = await p.eval(shift), midFade = await p.eval(fade);
  check(Math.abs(midShift - 120) <= 12 && midFade < 0.9, `pictures: a drag down moves the picture with the finger and the page shows through (${midShift} px, background ${midFade})`);
  if (shots) await p.scale(2);
  await p.shot("picture-viewer-drag");
  await p.shot("picture-viewer-drag", true);
  if (shots) await p.scale(3);
  await touch("touchEnd", [], 120);
  const sprung = await p.until(`${shift} === 0`, 2000);
  check(sprung === undefined ? false : !(await p.eval(viewerGone)), `pictures: a short drag springs back and the viewer stays (${await p.eval(shift)} px)`);
  await swipe(line(195, 300, 195, 300 + 844 / 4 + 40, 16));
  check(await p.until(`${viewerGone} && window.__revoked.includes(${JSON.stringify(dragged)})`, 2000) === true,
    "pictures: a drag past a quarter of the screen closes the viewer and lets the full picture go");
  check(await p.eval("document.activeElement?.matches(\"button.picture[data-index='1']\")") === true, "pictures: after a drag closes it, focus is back on the picture");
  check(await opened(), "pictures: the viewer opens for a flick");
  await swipe(line(195, 500, 195, 410, 3), { ms: 16, lift: false });
  await touch("touchEnd", [], 16);
  check(await p.until(viewerGone, 2000) === true, "pictures: a quick flick up closes the viewer though it is short");
  // A drag that closes right after one that sprang back: the spring's late end does not cut the slide out short.
  // The second drag starts while the spring back runs and lets go 100 to 300 ms after it began (touch times do not
  // move timers): the spring's end, by its event or its timer, comes during the slide out. The observer reads each
  // change itself, so one the closing viewer had just before it left counts too; it is set before the first drag, so
  // no round trip sits between the two. The page times both lifts; when the browser was too slow to let go before the
  // spring's 350 ms timer (old code would pass), it is set up again.
  let gap = Infinity;
  for (let tries = 0; tries < 3 && gap >= 300; tries++) {
    check(await opened(), "pictures: the viewer opens for two drags");
    await p.eval("window.__lifts = []; window.__cut = false; { const v = document.querySelector('.viewer'); v.addEventListener('touchend', () => window.__lifts.push(performance.now()), true); new MutationObserver((rs) => { for (const r of rs) if (r.target.classList.contains('closing') && !r.target.classList.contains('moving')) window.__cut = true; }).observe(v, { attributes: true, attributeFilter: ['class'] }); }");
    await swipe(line(195, 300, 195, 360, 2));
    const sprang = Date.now();
    // Sent without waiting on each answer, so slow answers from the busy page do not stretch the gap.
    const sent = [touch("touchStart", [{ id: 1, x: 195, y: 300 }], 0), touch("touchMove", [{ id: 1, x: 195, y: 600 }], 16)];
    await new Promise((r) => setTimeout(r, Math.max(0, 100 - (Date.now() - sprang))));
    await Promise.all([...sent, touch("touchEnd", [], 16)]);
    gap = await p.eval("window.__lifts.length === 2 ? Math.round(window.__lifts[1] - window.__lifts[0]) : Infinity");
    if (!(await p.until(viewerGone, 2000))) break;
  }
  check(gap < 300, `pictures: the drag after a spring back lets go before the spring's timer (${gap} ms after it)`);
  check(await p.eval(viewerGone) === true && await p.eval("window.__cut") === false, "pictures: a drag that closes right after a spring back slides out whole");
  check(await opened(), "pictures: the viewer opens for a sideways drag");
  await swipe(line(60, 420, 330, 440, 16));
  await new Promise((r) => setTimeout(r, 400));
  check(!(await p.eval(viewerGone)) && await p.eval(shift) === 0, `pictures: a sideways drag does not move or close the viewer (${await p.eval(shift)} px)`);
  await swipe(line(150, 300, 150, 600, 16), { second: line(240, 300, 240, 600, 16) });
  await new Promise((r) => setTimeout(r, 400));
  check(!(await p.eval(viewerGone)) && await p.eval(shift) === 0, `pictures: two fingers (a pinch) do not close the viewer (${await p.eval(shift)} px)`);
  // A real tap that moves the finger a little is still a tap: it zooms the picture, and on Close it closes.
  const tap = async (sel) => {
    const [x, y] = await p.eval(`(() => { const b = document.querySelector(${JSON.stringify(sel)}).getBoundingClientRect(); return [b.x + b.width / 2, b.y + b.height / 2]; })()`);
    await swipe([[x, y], [x + 2, y + 4]]);
  };
  // Chrome keeps moves inside its touch slop from the page, so the tap is also checked with a move the page sees:
  // under the drag threshold it is not cancelled (Safari would drop the click).
  check(await p.eval(`(() => {
    const v = document.querySelector('.viewer'), at = (y) => [new Touch({ identifier: 1, target: v, clientX: 195, clientY: y })];
    v.dispatchEvent(new TouchEvent('touchstart', { touches: at(400), changedTouches: at(400), bubbles: true, cancelable: true }));
    const kept = v.dispatchEvent(new TouchEvent('touchmove', { touches: at(406), changedTouches: at(406), bubbles: true, cancelable: true }));
    v.dispatchEvent(new TouchEvent('touchend', { touches: [], changedTouches: at(406), bubbles: true, cancelable: true }));
    return kept;
  })()`) === true, "pictures: a move under the drag threshold is not cancelled");
  await tap(".viewer img");
  check(await p.until("!!document.querySelector('.viewer img.zoomed')", 1000) === true, "pictures: a tap that moves a little still zooms the picture");
  await tap(".viewer img");
  await p.until("!document.querySelector('.viewer img.zoomed')", 1000);
  await tap(".viewer-close");
  check(await p.until(viewerGone, 1000) === true, "pictures: a tap on Close that moves a little still closes the viewer");
  check(await opened(), "pictures: the viewer opens to zoom in");
  await p.click(".viewer img");   // zoomed in to its own size
  await swipe(line(195, 300, 195, 600, 16));
  await new Promise((r) => setTimeout(r, 400));
  check(!(await p.eval(viewerGone)) && await p.eval("!!document.querySelector('.viewer img.zoomed')"), "pictures: zoomed in, a drag pans and does not close the viewer");
  await p.click(".viewer-close");
  // Reduced motion: past the threshold it closes at the lift, with no slide out.
  await p.cdp("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: "reduce" }] });
  check(await opened(), "pictures: the viewer opens with reduced motion");
  await swipe(line(195, 300, 195, 300 + 844 / 4 + 40, 16));
  check(await p.eval(viewerGone) === true, "pictures: with reduced motion a long drag closes at once");
  await p.cdp("Emulation.setEmulatedMedia", { features: [] });
  // Scrolled far away: the previews are let go.
  const previews = await p.eval("[...document.querySelectorAll('button.picture img')].map((i) => i.src)");
  await p.eval("window.dispatchEvent(new WheelEvent('wheel', { deltaY: 100 })); window.scrollTo(0, document.documentElement.scrollHeight)");
  check(await p.until(`!document.querySelector('button.picture img') && ${JSON.stringify(previews)}.every((u) => window.__revoked.includes(u))`, 4000) === true,
    "pictures: let go when they leave the view");
  // Another pane while they are far from the view: the boxes that left the page are not watched any more.
  await p.eval(`window.__oldBox = ${boxes}[0]`);
  await p.eval("location.hash = '#pane=w2%3Ap1'");
  await p.until("document.querySelector('section.conversation')?.dataset.pane === 'w2:p1'", 8000);
  const stale = "[...window.__observed].filter((b) => !b.isConnected).length";
  check(await p.until(`${stale} === 0`, 2000) === true, `pictures: boxes out of the page are not watched (${await p.eval(stale)})`);
  // An entry the observer had queued before the box left comes late: the box is not asked for or kept.
  const before = await p.eval("window.__pics.length");
  await p.eval("window.__pictureIO.__cb([{ target: window.__oldBox, isIntersecting: true }], window.__pictureIO)");
  await new Promise((r) => setTimeout(r, 500));
  check(await p.eval("window.__pics.length") === before, `pictures: a late entry for a box out of the page -> not asked for (${before} -> ${await p.eval("window.__pics.length")})`);
  await p.eval("location.hash = '#pane=w2%3Ap2'");
  check(await p.until(`${boxes}.length === 3`, 8000) === true, "pictures: back to the pane");
  // The pane gets another session while the full picture is open: the viewer closes and lets it go.
  await p.eval(`window.dispatchEvent(new WheelEvent('wheel', { deltaY: -100 })); ${boxes}[1].scrollIntoView({ block: 'start' })`);
  await p.click("button.picture[data-index='1']");
  check(await p.until("document.querySelector('.viewer img')?.naturalWidth === 1600", 8000) === true, "pictures: the viewer opens again");
  const again = await p.eval("document.querySelector('.viewer img').src");
  hub.stdin.write("session w2:p2 s21\n");
  check(await p.until(`!document.querySelector('.viewer') && window.__revoked.includes(${JSON.stringify(again)})`, 8000) === true,
    "pictures: another session in the pane closes the viewer and lets the full picture go");
  // The agent's session ends while the full picture is open (no new conversation comes): the viewer closes and lets it go.
  hub.stdin.write("session w2:p2 s20\n");   // the pictures again (a new conversation and snapshot)
  check(await p.until(`${boxes}.length === 3`, 8000) === true, "pictures: back to the session with pictures");
  await p.eval(`window.dispatchEvent(new WheelEvent('wheel', { deltaY: -100 })); ${boxes}[1].scrollIntoView({ block: 'start' })`);
  await p.click("button.picture[data-index='1']");
  check(await p.until("document.querySelector('.viewer img')?.naturalWidth === 1600", 8000) === true, "pictures: the viewer opens");
  const ended = await p.eval("document.querySelector('.viewer img').src");
  hub.stdin.write("nosession w2:p2\n");
  check(await p.until(`!document.querySelector('.viewer') && window.__revoked.includes(${JSON.stringify(ended)})`, 8000) === true,
    "pictures: the session ends -> the viewer closes and lets the full picture go");
  await p.close();
  pages.splice(pages.indexOf(p), 1);
  check(p.errors.length === 0, `pictures: no errors or CSP violations (${p.errors.join(" | ")})`);
  hub.stdin.write("session w2:p2 s4\n");
}


/** An agent team took the pane's log: the same two notes as the Mac, from the snapshot's `team`. */
async function team() {
  phase("agent teams");
  const p = await page(await context());
  pages.push(p);
  await p.goto(origin + "/");
  await p.eval("localStorage.setItem('flockbook.token', 'input-token')");
  await p.reload();
  await p.until("!!document.querySelector('.list')", 8000);
  await p.eval("location.hash = '#pane=w2%3Ap2'");
  hub.stdin.write("team w2:p2 waiting\n");
  check(await p.until("document.querySelector('.team-wait h2')?.textContent === \"Waiting for the lead's log\"", 8000) === true
    && await p.eval("document.querySelector('.team-wait p').textContent") === "Herdr points this pane at an agent-team teammate. The lead's conversation shows here once Herdr reports it. The terminal shows what it is doing."
    && await p.eval("!document.querySelector('.team-line') && !document.querySelector('.items')?.textContent.includes('Loading')"),
    "team: no lead log -> Waiting for the lead's log, not Loading");
  await p.shot("team-waiting");
  await p.shot("team-waiting", true);
  hub.stdin.write("team w2:p2 last s4\n");
  check(await p.until("document.querySelector('.team-line')?.textContent === \"Agent team: showing the lead's last known session.\"", 8000) === true
    && await p.eval("!document.querySelector('.team-wait')"), "team: the last lead -> one line over the conversation");
  check(await p.eval(noScroll), "team: no sideways scroll");
  check(await p.eval("(() => { const l = document.querySelector('.team-line'); return l.scrollWidth <= l.clientWidth; })()") === true,
    "team: the line is read whole at 390 pt (not cut off)");
  await p.shot("team-lastlead");
  await p.shot("team-lastlead", true);
  hub.stdin.write("team w2:p2 none s4\n");
  check(await p.until("!document.querySelector('.team-line') && !document.querySelector('.team-wait')", 8000) === true, "team: herdr's own log -> no note");
  await p.close();
  pages.splice(pages.indexOf(p), 1);
  check(p.errors.length === 0, `team: no errors or CSP violations (${p.errors.join(" | ")})`);
}
