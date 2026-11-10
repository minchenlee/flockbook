<!-- Each user-visible change adds one line here, in the same branch. At release this file becomes vX.Y.Z.md. -->
## What's New in vX.Y.Z

One short line: what this release is about.

### ✨ New

- Herdr app mod: type "/" in the reply box of a Claude Code chat to see its commands: your own commands, skills, plugins, and `/clear`, `/compact`, `/context`. ↑ ↓ choose, Tab or Enter completes the name, Esc closes. Needs the Flockbook helper of this release.

- Herdr app mod: right-click a message in the chat and choose "Copy message" to copy its whole text as Markdown. It works when selecting text and ⌘C copy nothing (a drag across a code box, or a reply that is still being written).

### 🔒 Security

### 🐛 Bug Fixes

- Herdr app mod: the "/" list in a Claude Code chat scrolls past 8 rows (↑ ↓ or the wheel; "4–11 of 20" says where you are), and lists the common built-ins too: `/model` and `/effort` (then their values: `/model sonnet`), `/resume`, `/review`, `/init`, `/rewind`, `/usage` and more. A row says "opens a screen" when the command, sent alone, opens a screen in the terminal.

- Herdr app mod: a message you sent with pictures shows whole. Its "2 images" line was not counted in the row's height, so the bottom of the bubble was cut off.
- Herdr app mod: a code block fenced with `~~~`, or with more words after its language (`` ```js title=a.js ``), shows whole. Before, the first was cut off at the bottom, and the second drew the rest of the message as code.

### ⚙️ Changes

- Herdr app mod: the "working" spinner turns more smoothly: a short arc over a faint ring, one turn in 0.9 s, like Herdr's own.
