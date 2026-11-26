// Links in the chat (Resources P0): the helper's `links` on an assistant row become Markdown links, so TextView
// draws them and opens them itself (http/https only). The helper decides where (each link's `block`, mdsplit.rs);
// the mod only writes them. Synthetic texts; nothing here opens a URL.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { linkHref, linkedMarkdown, markdownView } from "../mods/flockbook/mdblocks.js";

// The cases the helper and the mod share (xplat/flockbook-modfeed/tests/md_links.rs checks the helper's side).
const CASES = JSON.parse(readFileSync(new URL("../../Tests/Fixtures/resources/md-links.json", import.meta.url), "utf8"));

const u16 = (s) => s.length;   // JS strings count UTF-16 units, as the helper's offsets do
// A link the helper judged plain text in the block at `block` (0: the text is one block).
const at = (text, target, res = "link", block = 0) => { const start = text.indexOf(target); return { start, end: start + u16(target), res, block }; };

test("a bare URL becomes a link with the same visible text", () => {
  const t = "PR: https://github.com/o/r/pull/9 ok";
  assert.equal(linkedMarkdown(t, [at(t, "https://github.com/o/r/pull/9", "pr")]), "PR: [https://github.com/o/r/pull/9](https://github.com/o/r/pull/9) ok");
});

test("offsets are UTF-16: emoji and Chinese around the link", () => {
  const t = "🎉 見 https://x.com/a，好 👍";
  const l = { start: 5, end: 20, res: "link", block: 0 };
  assert.equal(t.slice(l.start, l.end), "https://x.com/a");
  assert.equal(linkedMarkdown(t, [l]), "🎉 見 [https://x.com/a](https://x.com/a)，好 👍");
});

test("files are not links yet (no way to open them in P0); text stays as it is", () => {
  const t = "Wrote /home/me/plan.md and ~/r/a.html";
  assert.equal(linkedMarkdown(t, [at(t, "/home/me/plan.md", "doc"), at(t, "~/r/a.html", "doc")]), t);
});

test("the address is written so it opens exactly what the helper found: & and \\ escaped", () => {
  const u = "https://x.com/?a=1&amp;b=2&copy=3";
  const t = `go ${u}`;
  assert.equal(linkedMarkdown(t, [at(t, u)]), `go [${u}](https://x.com/?a=1\\&amp;b=2\\&copy=3)`);
  const p = "https://x.com/a(b)&amp;";
  assert.equal(linkedMarkdown(p, [at(p, p)]), `[${p}](<https://x.com/a(b)\\&amp;>)`);
});

test("fences: the chat and the helper agree on what opens one", async () => {
  const { splitBlocks } = await import("../mods/flockbook/blocks.js");
  assert.deepEqual(splitBlocks("```x``` then https://x.com").map((b) => b.k), ["md"]);
  assert.deepEqual(splitBlocks("~~~ a`b\nhttps://x.com\n~~~").map((b) => b.k), ["code"]);
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
  assert.equal(linkedMarkdown(t, [{ start: 90, end: 99, res: "link", block: 0 }, { start: 3, end: 3, res: "link", block: 0 }]), t);
  assert.equal(linkedMarkdown(t, [b, a, { start: a.start + 2, end: a.end + 4, res: "link", block: 0 }]),
    "a [https://a.com](https://a.com) b [https://b.com](https://b.com)");
  assert.equal(linkedMarkdown(t, null), t);
  assert.equal(linkedMarkdown(t, "nope"), t);
});

const T = new Proxy({}, { get: (_t, k) => `#${k}` });
const views = (el) => { const out = []; (function walk(n) { if (!n) return; if (n.tag === "md") out.push(n._a); (n.kids ?? []).forEach(walk); })(el); return out; };

test("markdownView: links in prose blocks at their place in the whole text; code boxes and tables untouched", async () => {
  const text = "Intro https://a.com\n\n```sh\ncurl https://b.com\n```\n| H |\n|---|\n| https://c.com |\n\nEnd https://d.com";
  const { splitBlocks } = await import("../mods/flockbook/blocks.js");
  const blocks = [0, null, null, splitBlocks(text).at(-1).start];
  const links = ["https://a.com", "https://b.com", "https://c.com", "https://d.com"].map((u, i) => at(text, u, "link", blocks[i]));
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

// The cases for where a link may be written (code spans, indented lines, quotes, CRLF, escapes; Resources P0 fix1–5)
// are in md-links.json now.
// Resources mod polish (hg-res-mod-parser): the helper says where a link is plain text; the mod no longer guesses.
test("a link without `block` (an older helper, a cached row) stays text", () => {
  const t = "PR: https://github.com/o/r/pull/9 ok";
  const { block, ...l } = at(t, "https://github.com/o/r/pull/9", "pr");
  assert.equal(linkedMarkdown(t, [l]), t);
  assert.equal(views(markdownView("r", t, T, 600, 1, [l]))[0].text, t);
});

test("a link is written only in the block its `block` names", async () => {
  const text = "Intro https://a.com\n\n```sh\nx\n```\nEnd https://d.com";
  const { splitBlocks } = await import("../mods/flockbook/blocks.js");
  const end = splitBlocks(text).at(-1).start;
  const wrong = [at(text, "https://a.com", "link", end), at(text, "https://d.com", "link", 0)];
  for (const x of views(markdownView("r", text, T, 600, 1, wrong))) assert.ok(!x.text.includes("]("), x.text);
});

test("the shared cases: the mod writes exactly the links the helper marked", () => {
  assert.ok(CASES.length > 40);
  for (const c of CASES) {
    assert.ok(Array.isArray(c.links), c.name + ": no links (run MD_LINKS_UPDATE=1 in md_links.rs)");
    const plain = JSON.stringify(views(markdownView("r", c.text, T, 600)));
    const written = c.links.filter((l) => JSON.stringify(views(markdownView("r", c.text, T, 600, 1, [l]))) !== plain)
      .map((l) => c.text.slice(l.start, l.end));
    assert.deepEqual(written, c.wrapped, c.name);
  }
});
