// Fake gpui-kit and gpui-base for node tests. Every builder method returns the element, so any
// chain works; children and handlers are kept so tests can walk the tree and fire clicks.
export const calls = [];
const mk = (tag) => {
  const o = { tag, kids: [], _h: {}, _a: {} };
  const p = new Proxy(o, {
    get(t, k) {
      if (k in t) return t[k];
      if (typeof k === "symbol") return undefined;
      return (...a) => {
        if (k === "child") t.kids.push(a[0]);
        else if (k === "children") t.kids.push(...a[0]);
        else if (k.startsWith("on_") || k === "hover") t._h[k] = a[a.length - 1];
        else if (k === "when") return a[0] ? a[1](p) : p;
        else if (k === "track_scroll") t.track = a[0];
        else t._a[k] = a.length <= 1 ? a[0] : a;
        return p;
      };
    },
  });
  return p;
};
export const div = () => mk("div");
// Finds the first element (depth first) whose id is `id`.
export function byId(el, id) {
  if (!el || typeof el !== "object") return null;
  if (el._a && el._a.id === id) return el;
  for (const k of el.kids || []) { const f = byId(k, id); if (f) return f; }
  return null;
}
export const svg = (path) => { const e = mk("svg"); e._a.path = path; return e; };
export const image = (path) => { const e = mk("image"); e._a.path = path; return e; };
export const v_flex = () => mk("v");
export const h_flex = () => mk("h");
export class View {}
export const TextView = { markdown: (_id, text) => { const e = mk("md"); e._a.text = text; return e; } };
export const Scrollbar = { vertical: () => mk("sb") };
export const Textarea = { new: () => mk("ta") };
export const made = { textarea: 0 };
export const TextareaState = {
  new: () => { made.textarea++; let v = ""; const h = {}; return { value: () => v, set_value: (x) => { v = x; }, on: (e, f) => { h[e] = f; return true; }, focus() {}, _h: h }; },
};
export let lastList = null;
export const v_virtual_list = (id, n, sizes, key, render) => { const e = mk("vl"); e._args = { id, n, sizes, key, render }; lastList = e; return e; };
export const VirtualListScrollHandle = {
  new: () => ({ scroll_to_item: (i, s) => calls.push(["scroll_to_item", i, s]), scroll_to_bottom: () => calls.push(["bottom"]) }),
};

// All text in an element tree, for assertions.
export function textOf(el) {
  if (el == null) return "";
  if (typeof el === "string" || typeof el === "number") return String(el);
  const own = el._a && typeof el._a.text === "string" ? el._a.text : "";
  return [own, ...(el.kids || []).map(textOf)].filter(Boolean).join(" ");
}
