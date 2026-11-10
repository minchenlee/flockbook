// A fixed set of texts for the row height golden test (rowgolden.test.mjs). Do not change a text: its golden
// numbers are the heights the chat guesses today. Add new texts at the end, with their numbers.
export const TEXTS = {
  short: "hi",
  latin: "The helper reads the log on that machine and sends the lines back as they are. ".repeat(4).trim(),
  latinParas: Array.from({ length: 3 }, (_v, i) => `Paragraph ${i + 1}: the mirror keeps the tail and fetches older pages on demand.`).join("\n\n"),
  softBreaks: "Line one of a paragraph\nline two joins it\nline three joins it too",
  cjk: "目前還有兩個問題，使用者在地圖外面放開手指時，程式可能收不到結束事件。".repeat(3),
  cjkReply: [
    "目前還有兩個問題",
    "",
    "- 使用者在地圖外面放開手指時，程式可能收不到結束事件。要用 `on_pointer_cancel_out` 這類事件補上，讓拖動一定會停下。",
    "- 每張卡片寬度都不同，要先知道它們的實際位置，分隔線才放得準。",
    "",
    "範圍： 我建議先做上方的 card 列。左邊的 folder，系統自己已經能拖動排序（說明裡有寫）。",
  ].join("\n"),
  inlineCode: Array.from({ length: 12 }, (_v, i) => `\`on_event_name_${i}\``).join(" "),
  nestedList: ["- one", "  - one a", "  - one b", "- two", "1. first", "2. second"].join("\n"),
  listAfterPara: "Steps:\n- build\n- test\n- ship",
  heading: "# Title\n\n## Section\n\nBody text under the section.",
  quote: "> a quoted line\n> and another one that joins it",
  rule: "Above\n\n---\n\nBelow",
  table: ["| Name | Value |", "| --- | ---: |", "| width | 748 |", "| a much longer cell that wraps around at least once in the column | 2 |"].join("\n"),
  tableInText: ["Before the table.", "", "| a | b |", "|---|---|", "| 1 | 2 |", "", "After the table."].join("\n"),
  code: ["Run this:", "", "```sh", "swift build", "swift run HerdrKitChecks", "```", "", "Then look."].join("\n"),
  codeLong: ["```js", "const x = " + "\"a long string that wraps in the box\" + ".repeat(4) + "1;", "```"].join("\n"),
  twoCodes: ["```", "a", "```", "between", "```py", "b", "c", "```"].join("\n"),
  codeUnclosed: ["Start", "```", "never closed", "still code"].join("\n"),
  // Split differently by the estimate and the drawing before both used blocks.js.
  tildeFence: ["Run:", "", "~~~", "one", "two", "three", "~~~"].join("\n"),
  fenceInfo: ["```js title=a.js", "const a = 1;", "```", "", "after"].join("\n"),
};

// Texts the estimate and the drawing split into different blocks (their test is a TODO). None now.
export const KNOWN_MISMATCH = new Set();

export const WIDTHS = [748, 400];

// Rows of every kind, closed and open where a row opens. [name, row, open]
export const ROWS = [
  ["user", { k: "user", text: TEXTS.cjkReply }, false],
  ["user short", { k: "user", text: TEXTS.short }, false],
  ["user images", { k: "user", text: TEXTS.short, images: 2 }, false],
  ["user only images", { k: "user", text: "", images: 1 }, false],
  ["note", { k: "user", from: "t2 docs-worker", text: TEXTS.latinParas }, false],
  ["note open", { k: "user", from: "t2 docs-worker", text: TEXTS.latinParas }, true],
  ["assistant", { k: "assistant", text: TEXTS.code }, false],
  ["assistant table", { k: "assistant", text: TEXTS.table }, false],
  ["tools", { k: "tools", chips: [{ calls: [{}, {}] }, { calls: [{}] }] }, false],
  ["tools open", { k: "tools", chips: [{ calls: [{}, {}] }, { calls: [{}] }] }, true],
  ["sep", { k: "sep" }, false],
  ["footer", { k: "footer" }, false],
  ["status", { k: "status" }, false],
  ["perm", { k: "perm" }, false],
  ["question", { k: "question" }, false],
  ["other", { k: "other" }, false],
];
