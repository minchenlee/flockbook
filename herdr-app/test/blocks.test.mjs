// The one block parser (blocks.js) that the drawing and the row height estimate share: fences as in CommonMark.
import { test } from "node:test";
import assert from "node:assert/strict";
import { splitBlocks } from "../mods/flockbook/blocks.js";

const codes = (text) => splitBlocks(text).filter((b) => b.k === "code").map((b) => [b.lang, b.text]);

test("a ~~~ fence is a code box", () => {
  assert.deepEqual(codes("~~~\none\n~~~"), [["", "one"]]);
  assert.deepEqual(codes("~~~py\none\n~~~"), [["py", "one"]]);
});

test("an opening fence may have more words after the language", () => {
  assert.deepEqual(codes("```js title=a.js\nconst a = 1;\n```\nafter"), [["js", "const a = 1;"]]);
  assert.deepEqual(splitBlocks("```js title=a.js\nx\n```\nafter").map((b) => b.k), ["code", "md"]);
});

test("a closing fence is the same character, as long or longer, with nothing after it", () => {
  assert.deepEqual(codes("```\na\n```py\nb\n```"), [["", "a\n```py\nb"]]);   // "```py" inside does not close
  assert.deepEqual(codes("~~~\na\n```\nb\n~~~"), [["", "a\n```\nb"]]);        // a backtick line inside ~~~
  assert.deepEqual(codes("````\na\n```\nb\n````"), [["", "a\n```\nb"]]);      // a shorter fence inside a longer one
  assert.deepEqual(codes("```\na\n`````"), [["", "a"]]);                       // a longer one closes
});

test("a backtick fence's info has no backtick: ```a``` on one line is inline code, not a box", () => {
  assert.deepEqual(splitBlocks("```a``` text").map((b) => b.k), ["md"]);
});

test("a code box keeps its source lines, an empty box has none", () => {
  assert.deepEqual(splitBlocks("```\n```")[0].lines, []);
  assert.deepEqual(splitBlocks("```\n\n```")[0].lines, [""]);
  assert.deepEqual(splitBlocks("```\na\r\n\tb\n```")[0].lines, ["a\r", "\tb"]);
});
