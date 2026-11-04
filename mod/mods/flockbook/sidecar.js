// Calls the sidecar (docs/protocol.md): one command per exec, JSON Lines back.
import { exec, appHome } from "herdr";

export const PROTOCOL = 1;
export const sidecarPath = () => `${appHome()}/flockbook/bin/flockbook-modfeed`;
export const cachePath = () => `${appHome()}/flockbook/cache.json`;

// Splits an answer into its header, end, error and the lines between. Unknown types stay in `lines`.
export function parseAnswer(stdout) {
  const out = { header: null, end: null, error: null, lines: [] };
  for (const raw of String(stdout || "").split("\n")) {
    if (!raw.trim()) continue;
    let o; try { o = JSON.parse(raw); } catch { continue; }
    if (o.type === "header") out.header = o;
    else if (o.type === "end") out.end = o;
    else if (o.type === "error") out.error = o;
    else out.lines.push(o);
  }
  return out;
}

// Runs one command. Always resolves: a broken or missing sidecar comes back as `error`.
export async function call(cmd, args = []) {
  let r;
  try { r = await exec(sidecarPath(), [cmd, ...args]); }
  catch (e) { return { header: null, end: null, lines: [], error: { code: "no-sidecar", message: `The Flockbook sidecar could not start (${e}).` } }; }
  const a = parseAnswer(r && r.stdout);
  if (!a.header || !a.end) {
    const why = (r && (r.stderr || "").trim()) || `exit ${r ? r.code : "?"}`;
    a.error = a.error || { code: r && r.code === 127 ? "no-sidecar" : "sidecar", message: `The Flockbook sidecar did not answer: ${why}` };
    return a;
  }
  if (a.header.protocol !== PROTOCOL) {
    a.error = { code: "protocol", message: `The sidecar speaks protocol ${a.header.protocol}; this mod needs ${PROTOCOL}. Update both together.` };
  }
  return a;
}

// The commands, with the cache file the sidecar keeps.
export const feed = (terminal, cursor) => call("feed", ["--terminal", terminal, "--cache", cachePath(), ...(cursor ? ["--cursor", cursor] : [])]);
export const older = (terminal, before, path) => call("feed", ["--terminal", terminal, "--before", String(before), "--path", path]);
export const status = (terminal) => call("status", ["--terminal", terminal, "--cache", cachePath()]);
export const card = (terminal, all = false) => call("card", ["--terminal", terminal, ...(all ? ["--all"] : []), "--cache", cachePath()]);
export const approve = (terminal, request, allow) => call("approve", ["--terminal", terminal, "--request", request, allow ? "--allow" : "--deny", "--cache", cachePath()]);
export const answer = (terminal, request, answers) => call("answer", ["--terminal", terminal, "--request", request, "--answers", JSON.stringify(answers), "--cache", cachePath()]);
// Only after the user pressed Install on the card; `sha256` is the one the card showed.
// `--cache`: the sidecar remembers machines beside it (a platform, a missing helper), and install or remove resets that.
export const installHelper = (machine, sha256) => call("helper", ["install", "--machine", machine, "--sha256", sha256, "--cache", cachePath()]);
export const removeHelper = (machine) => call("helper", ["remove", "--machine", machine, "--cache", cachePath()]);
export const pasteImage = () => call("paste-image", ["--dir", `${appHome()}/flockbook/paste`]);
// `seen`: the agent kind and the log the chat last showed; the sidecar refuses another agent or conversation.
export const send = (terminal, text, files = [], seen = null) => call("send", ["--terminal", terminal, "--text", text,
  ...files.flatMap((f) => ["--file", f]),
  ...(seen?.agent ? ["--expect-kind", seen.agent] : []), ...(seen?.path ? ["--expect-path", seen.path] : [])]);

// The sidecar's "changed" reply when another agent took the pane: nothing was sent.
export const AGENT_CHANGED = "Another agent is in this pane now. Nothing was sent.";
export const agentChanged = (done) => done?.result === "changed" && /another agent/i.test(done?.message ?? "");
