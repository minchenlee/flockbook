// Fake "herdr" module. Tests set the live data in `fake` and read what the mod asked for in `log`.
export const fake = { spaces: [], agents: [], panes: [], focused: null, shared: {}, home: "/home/test", appHome: "/home/test/.config/herdr-app", exec: null,
  layout: null, window: { platform: "macos", titlebar: "none", buttons: false, maximized: false, fullscreen: false, active: true }, settings: { zoom: 1 } };
export const log = { run: [], exec: [], notify: [], shared: [], overlays: [], order: [] };
export function reset() { Object.assign(fake, { props: {}, spaces: [], agents: [], panes: [], focused: null, shared: {}, exec: null, layout: null }); for (const k of Object.keys(log)) log[k] = []; }
export const theme = () => new Proxy({}, { get: (_t, k) => (typeof k === "string" ? `#${k}` : undefined) });
export const spaces = () => fake.spaces;
export const agents = () => fake.agents;
export const panes = () => fake.panes;
export const terminals = () => [];
export const focused = () => fake.focused;
export const home = () => fake.home;
export const appHome = () => fake.appHome;
export const props = () => fake.props ?? {};
export const shared = (k) => fake.shared[k];
export const setShared = (k, v) => { fake.shared[k] = v; log.shared.push([k, v]); return null; };
export const run = (name, args) => { log.run.push([name, args]); return null; };
export const notify = (n) => { log.notify.push(n); return null; };
export const setAgentOrder = (list) => { log.order = list; return null; };
export const layout = () => fake.layout;
export const window = () => fake.window;
export const settings = () => fake.settings;
export const nextCommand = () => new Promise(() => {});
export const watchFolder = () => new Promise(() => {});
export const exec = async (cmd, args = [], opts) => {
  log.exec.push([cmd, args, opts]);
  return fake.exec ? fake.exec(cmd, args, opts) : { code: 127, stdout: "", stderr: "no exec in tests" };
};
export const hc = (args) => exec("hc", args);
export const readTextFile = async (p) => (fake.readText ? fake.readText(p) : null);
export const writeTextFile = async () => null;
export const pickFolder = async () => null;
export const playSound = () => null;
export const openOverlay = (spec) => { log.overlays.push(spec); return "o1"; };
export const closeOverlay = (id) => { (log.closed ??= []).push(id); return null; };
