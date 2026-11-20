// Links in the chat (Resources P0): the helper's `links` on an assistant row become Markdown links, so TextView
// draws them and opens them itself (http/https only). Synthetic texts; nothing here opens a URL.
import { test } from "node:test";
import assert from "node:assert/strict";
import { linkHref, linkedMarkdown, markdownView } from "../mods/flockbook/mdblocks.js";

const u16 = (s) => s.length;   // JS strings count UTF-16 units, as the helper's offsets do
const at = (text, target, res = "link") => { const start = text.indexOf(target); return { start, end: start + u16(target), res }; };

test("a bare URL becomes a link with the same visible text", () => {
  const t = "PR: https://github.com/o/r/pull/9 ok";
  assert.equal(linkedMarkdown(t, [at(t, "https://github.com/o/r/pull/9", "pr")]), "PR: [https://github.com/o/r/pull/9](https://github.com/o/r/pull/9) ok");
});

test("offsets are UTF-16: emoji and Chinese around the link", () => {
  const t = "🎉 見 https://x.com/a，好 👍";
  const l = { start: 5, end: 20, res: "link" };
  assert.equal(t.slice(l.start, l.end), "https://x.com/a");
  assert.equal(linkedMarkdown(t, [l]), "🎉 見 [https://x.com/a](https://x.com/a)，好 👍");
});

test("files are not links yet (no way to open them in P0); text stays as it is", () => {
  const t = "Wrote /home/me/plan.md and ~/r/a.html";
  assert.equal(linkedMarkdown(t, [at(t, "/home/me/plan.md", "doc"), at(t, "~/r/a.html", "doc")]), t);
});

test("a URL already in a Markdown link, an autolink or code is left alone", () => {
  const md = "[the PR](https://github.com/o/r/pull/1)";
  assert.equal(linkedMarkdown(md, [at(md, "https://github.com/o/r/pull/1", "pr")]), md);
  const angle = "[a](<https://x.com/a(b)>)";
  assert.equal(linkedMarkdown(angle, [at(angle, "https://x.com/a(b)")]), angle);
  const auto = "see <https://x.com>";
  assert.equal(linkedMarkdown(auto, [at(auto, "https://x.com")]), auto);
  const code = "Open `https://x.com` now";
  assert.equal(linkedMarkdown(code, [at(code, "https://x.com")]), code);
  const padded = "Open ` https://x.com ` now";
  assert.equal(linkedMarkdown(padded, [at(padded, "https://x.com")]), padded);
});

test("the address it opens: a scheme for bare hosts; files and anything else none", () => {
  assert.equal(linkHref({ res: "local" }, "localhost:3000"), "http://localhost:3000");
  assert.equal(linkHref({ res: "artifact" }, "claude.ai/artifact/x"), "https://claude.ai/artifact/x");
  assert.equal(linkHref({ res: "pr" }, "github.com/o/r/pull/3"), "https://github.com/o/r/pull/3");
  assert.equal(linkHref({ res: "link" }, "http://x.com"), "http://x.com");
  assert.equal(linkHref({ res: "doc" }, "/home/me/a.md"), null);
  assert.equal(linkHref({ res: "link" }, "javascript:alert(1)"), null);
  assert.equal(linkHref({ res: "link" }, "file:///etc/passwd"), null);
  assert.equal(linkHref({ res: "what" }, "https://x.com"), null, "an unknown kind");
});

test("markup in the visible text is escaped; parentheses in the address are wrapped", () => {
  const t = "see https://en.wikipedia.org/wiki/Foo_(bar)_baz";
  const out = linkedMarkdown(t, [at(t, "https://en.wikipedia.org/wiki/Foo_(bar)_baz")]);
  assert.equal(out, "see [https://en.wikipedia.org/wiki/Foo\\_(bar)\\_baz](<https://en.wikipedia.org/wiki/Foo_(bar)_baz>)");
});

test("bad ranges are skipped: out of the text, empty, overlapping, out of order", () => {
  const t = "a https://a.com b https://b.com";
  const a = at(t, "https://a.com"), b = at(t, "https://b.com");
  assert.equal(linkedMarkdown(t, [{ start: 90, end: 99, res: "link" }, { start: 3, end: 3, res: "link" }]), t);
  assert.equal(linkedMarkdown(t, [b, a, { start: a.start + 2, end: a.end + 4, res: "link" }]),
    "a [https://a.com](https://a.com) b [https://b.com](https://b.com)");
  assert.equal(linkedMarkdown(t, null), t);
  assert.equal(linkedMarkdown(t, "nope"), t);
});

const T = new Proxy({}, { get: (_t, k) => `#${k}` });
const views = (el) => { const out = []; (function walk(n) { if (!n) return; if (n.tag === "md") out.push(n._a); (n.kids ?? []).forEach(walk); })(el); return out; };

test("markdownView: links in prose blocks at their place in the whole text; code boxes and tables untouched", () => {
  const text = "Intro https://a.com\n\n```sh\ncurl https://b.com\n```\n| H |\n|---|\n| https://c.com |\n\nEnd https://d.com";
  const links = ["https://a.com", "https://b.com", "https://c.com", "https://d.com"].map((u) => at(text, u));
  const v = views(markdownView("r1", text, T, 600, 1, links));
  assert.deepEqual(v.map((x) => x.id), views(markdownView("r1", text, T, 600)).map((x) => x.id), "the same views, the same ids");
  assert.equal(v[0].text, "Intro [https://a.com](https://a.com)\n");
  assert.ok(!v.some((x) => x.text.includes("(https://b.com)")), "the code box is not rewritten");
  assert.ok(!v.some((x) => x.text.includes("(https://c.com)")), "table cells are not rewritten (their widths are guessed from the plain text)");
  assert.equal(v[v.length - 1].text.trim(), "End [https://d.com](https://d.com)");
});

test("markdownView: one block, and no links, stay as they were", () => {
  const t = "Just https://x.com";
  assert.equal(views(markdownView("r", t, T, 600, 1, [at(t, "https://x.com")]))[0].text, "Just [https://x.com](https://x.com)");
  assert.equal(views(markdownView("r", t, T, 600))[0].text, t);
});
