// Flockbook settings: <appHome>/flockbook/config.json, set in the Flockbook settings dialog.
// {"check_updates": true, "status_row": "show", "remote_helper": "ask"}. A missing file or key is the default;
// the install never creates the file. A file that is not JSON is the defaults too, and is left alone
// (the screen says so). The helper reads the same file (the mod passes --home) for check_updates and
// remote_helper. Kept in shared(), so every copy of this mod redraws when one changes it.
import { appHome, readTextFile, writeTextFile, shared, setShared } from "herdr";
import * as sidecar from "./sidecar.js";

export const KEY_CONFIG = "flockbook.config";    // { file, bad, text }: the parsed file, and the text it was read from
export const KEY_UPDATE = "flockbook.update";    // the helper's last `update` line
const KEY_UPDATE_AT = "flockbook.update.at";     // when the mod last asked (ms)
const KEY_UPDATE_SEEN = "flockbook.update.seen"; // the version whose line was seen in the open sidebar
export const DEFAULTS = { check_updates: true, status_row: "show", remote_helper: "ask" };
export const CHOICES = { check_updates: [true, false], status_row: ["show", "hide"], remote_helper: ["ask", "never"] };
export const configPath = () => `${appHome()}/flockbook/config.json`;

export function parseConfig(text) {
  if (text == null) return { file: {}, bad: false, text: null };
  try {
    const v = JSON.parse(text);
    if (v && typeof v === "object" && !Array.isArray(v)) return { file: v, bad: false, text };
  } catch { /* not JSON */ }
  return { file: {}, bad: true, text };
}

export function setting(key) {
  const v = shared(KEY_CONFIG)?.file?.[key];
  return CHOICES[key].includes(v) ? v : DEFAULTS[key];
}
export const configBad = () => shared(KEY_CONFIG)?.bad === true;

// Reads the file again (someone may have edited it); shares it only when it changed.
export async function loadConfig() {
  let text = null;
  try { text = await readTextFile(configPath()); } catch { text = null; }
  const c = parseConfig(text);
  const was = shared(KEY_CONFIG);
  if (!was || was.text !== c.text) setShared(KEY_CONFIG, c);
  return c;
}

// Sets one key and keeps the others. False when the file is not JSON (it stays as it is) or the write failed.
export async function setSetting(key, value) {
  for (let i = 0; i < 2; i++) {
    const c = await loadConfig();
    if (c.bad) return false;
    const file = { ...c.file, [key]: value };
    const text = JSON.stringify(file, null, 2) + "\n";
    // Against the text just read: a change made meanwhile is read again, not overwritten.
    try { await writeTextFile(configPath(), text, c.text); }
    catch { continue; }
    setShared(KEY_CONFIG, { file, bad: false, text });
    if (key === "check_updates") {
      setShared(KEY_UPDATE_AT, null);
      setShared(KEY_UPDATE, null);
      if (value) await pollUpdate();
    }
    return true;
  }
  return false;
}

const HOUR = 60 * 60 * 1000;
// Asks the helper whether a newer Flockbook is out, at most once an hour (it asks GitHub at most once a
// day and keeps the answer). Off: the helper is not even asked.
export async function pollUpdate(now = Date.now()) {
  const at = shared(KEY_UPDATE_AT);
  if (at != null && now - at < HOUR) return shared(KEY_UPDATE) ?? null;
  setShared(KEY_UPDATE_AT, now);
  if (!setting("check_updates")) { if (shared(KEY_UPDATE)) setShared(KEY_UPDATE, null); return null; }
  const r = await sidecar.updateCheck();
  const line = r.error ? null : r.lines.find((l) => l.type === "update") ?? null;
  setShared(KEY_UPDATE, line);
  return line;
}

// The update line to show, or null: only for a newer release that was not dismissed, and only while on.
export function updateShown() {
  const u = shared(KEY_UPDATE);
  return u?.state === "newer" && u.latest && setting("check_updates") ? u : null;
}

// The dot on the top bar's sidebar button: a line to show that the sidebar has not shown yet.
export const updateDot = () => { const u = updateShown(); return Boolean(u) && shared(KEY_UPDATE_SEEN) !== u.latest; };
// The sidebar opened (docked or floating): its line is seen, so the dot goes.
export function updateSeen() { const u = updateShown(); if (u && shared(KEY_UPDATE_SEEN) !== u.latest) setShared(KEY_UPDATE_SEEN, u.latest); }

// Not now: hidden at once; the helper keeps it hidden for that version until a newer one.
export async function dismissUpdate() {
  const u = updateShown();
  if (!u) return;
  setShared(KEY_UPDATE, { ...u, state: "dismissed" });
  await sidecar.updateCheck(u.latest);
}
