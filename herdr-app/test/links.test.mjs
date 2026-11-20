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

test("a URL between two code spans is not code", () => {
  const t = "`a` https://x.com `b`";
  assert.equal(linkedMarkdown(t, [at(t, "https://x.com")]), "`a` [https://x.com](https://x.com) `b`");
  const multi = "``a ` b`` https://x.com";
  assert.equal(linkedMarkdown(multi, [at(multi, "https://x.com")]), "``a ` b`` [https://x.com](https://x.com)");
});

test("an address after spaces in a Markdown link is left alone", () => {
  for (const t of ["[x]( https://x.com/a)", "[x](  <https://x.com/a>)", "[x](\nhttps://x.com/a)"]) {
    assert.equal(linkedMarkdown(t, [at(t, "https://x.com/a")]), t, t);
  }
});

test("a URL after ! is not rewritten: it would become a picture", () => {
  const t = "Wow!https://x.com/a";
  assert.equal(linkedMarkdown(t, [at(t, "https://x.com/a")]), t);
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

// Resources P0 fix4: a line indented 4 or more may be code when its block is drawn alone (splitBlocks cuts at a fence
// or a table, also in a list item); the helper reads the whole reply, so it may send a link there.
test("markdownView: a 4-space line after a fence or a table in a list item is not rewritten", () => {
  const fence = "- a\n  ```\n  x\n  ```\n    more https://x.com";
  const table = "- a\n  | h |\n  | --- |\n    more https://x.com";
  // The helper's offsets for these texts (resources.rs): 29..42 and 31..44.
  for (const [text, start] of [[fence, 29], [table, 31]]) {
    assert.equal(text.slice(start, start + 13), "https://x.com");
    const v = views(markdownView("r", text, T, 600, 1, [{ start, end: start + 13, res: "link" }]));
    assert.ok(!v.some((x) => x.text.includes("](https://x.com)")), JSON.stringify(v.map((x) => x.text)));
  }
});

test("a link on a line indented 4 or more (a tab counts to the next 4) is left alone; other lines are not", () => {
  for (const t of ["- item\n\n    https://x.com", "    https://x.com", "\thttps://x.com", "  \thttps://x.com", "x\n     see https://x.com"]) {
    assert.equal(linkedMarkdown(t, [at(t, "https://x.com")]), t, JSON.stringify(t));
  }
  const t = "   https://a.com\n    https://x.com";
  assert.equal(linkedMarkdown(t, [at(t, "https://a.com"), at(t, "https://x.com")]), "   [https://a.com](https://a.com)\n    https://x.com", "3 spaces is not 4");
});

// Resources P0 fix5 (Codex round 4).
test("an address after CRLF in a link's destination is left alone", () => {
  const t = "[Guide](\r\nhttps://x.com/guide)";
  assert.equal(linkedMarkdown(t, [at(t, "https://x.com/guide")]), t);
  const s = "[Guide]( \r\n <https://x.com/guide>)";
  assert.equal(linkedMarkdown(s, [at(s, "https://x.com/guide")]), s);
});

test("markdownView: a quoted 4-space line after a table or a fence is not rewritten", () => {
  const table = "> intro\n| h |\n| --- |\n>     curl https://x.com/guide";
  const fence = "> intro\n    ```\n    x\n    ```\n>     curl https://x.com/guide";
  for (const text of [table, fence]) {
    const v = views(markdownView("r", text, T, 600, 1, [at(text, "https://x.com/guide")]));
    assert.ok(!v.some((x) => x.text.includes("](https://x.com/guide)")), JSON.stringify(v.map((x) => x.text)));
  }
});

test("quote markers are taken off before the indent is counted", () => {
  for (const t of [">     https://x.com", "> >     https://x.com", "  >  >     https://x.com", ">\thttps://x.com"]) {
    assert.equal(linkedMarkdown(t, [at(t, "https://x.com")]), t, JSON.stringify(t));
  }
  for (const t of ["> https://x.com", ">    https://x.com", "> > https://x.com"]) {
    assert.equal(linkedMarkdown(t, [at(t, "https://x.com")]), t.replace("https://x.com", "[https://x.com](https://x.com)"), JSON.stringify(t));
  }
});

test("an escaped backtick does not open or close a code span", () => {
  const t = "\\` `https://x.com` now";
  assert.equal(linkedMarkdown(t, [at(t, "https://x.com")]), t, "the real span starts after the escaped one");
  const u = "`a\\` https://x.com";
  assert.equal(linkedMarkdown(u, [at(u, "https://x.com")]), "`a\\` [https://x.com](https://x.com)", "after the span, text either way");
  const two = "\\\\`https://x.com`";
  assert.equal(linkedMarkdown(two, [at(two, "https://x.com")]), two, "two backslashes: the backtick is not escaped");
  const esc = "\\`https://x.com\\` ok";
  assert.equal(linkedMarkdown(esc, [at(esc, "https://x.com")]), "\\`[https://x.com](https://x.com)\\` ok", "both escaped: text");
});

test("in a code span a backslash escapes nothing: `a\\` closes, the next span is code", () => {
  const t = "`a\\` `https://x.com`";
  assert.equal(linkedMarkdown(t, [at(t, "https://x.com")]), t);
});

test("a code span does not cross a blank line: a backtick at a paragraph's end is text", () => {
  const t = "`a\\` means `\n\n`https://x.com`";
  assert.equal(linkedMarkdown(t, [at(t, "https://x.com")]), t, "the second paragraph's span is code");
  const u = "a `\n \t\nsee https://x.com `b`";
  assert.equal(linkedMarkdown(u, [at(u, "https://x.com")]), "a `\n \t\nsee [https://x.com](https://x.com) `b`", "a line of spaces is blank too");
});
