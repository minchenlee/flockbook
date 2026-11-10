<!-- Each user-visible change adds one line here, in the same branch. At release this file becomes vX.Y.Z.md. -->
## What's New in vX.Y.Z

One short line: what this release is about.

### ✨ New

- Mac app: type "/" in the reply box of a Claude Code, Codex, pi or Antigravity CLI (agy) chat to see its commands, as in the Herdr app mod: ↑ ↓ or the wheel move through them 8 at a time ("4–11 of 20"), Tab, Enter or a click completes the name, Esc closes; after `/model `, `/effort ` or `/thinking ` the list offers the values with their names. A command that opens a screen in the terminal (`/model`, `/status`) is not sent alone, and a message is not sent when the agent's own input box already has text in it: the reply box keeps your text and says why.
- Mac app: right-click a message in the chat (yours or the agent's) and choose "Copy Message" to copy its whole text as Markdown, with nothing selected. On a code block it comes after "Copy Code".

- Herdr app mod: the "/" list above the reply box works in Codex, pi and Antigravity CLI (agy) chats too: their common built-ins, pi's prompt templates and `/skill:` commands, and agy's skills.

- Herdr app mod: type "/" in the reply box of a Claude Code chat to see its commands: your own commands, skills, plugins, and `/clear`, `/compact`, `/context`. ↑ ↓ choose, Tab or Enter completes the name, Esc closes. Needs the Flockbook helper of this release.

- Herdr app mod: right-click a message in the chat and choose "Copy message" to copy its whole text as Markdown. It works when selecting text and ⌘C copy nothing (a drag across a code box, or a reply that is still being written).

### 🔒 Security

### 🐛 Bug Fixes

- Herdr app mod: a message is not sent when the agent's own input box in the terminal already has text in it (Claude Code, Codex, pi, agy and Grok Build). Before, the message was added after that text: a "/" left in the box made "/status" into "//status", which the agent took as a prompt. Now the chat says to clear the box in the terminal first and keeps your text. The phone page does the same.

- Herdr app mod: the "/" list in a Claude Code chat scrolls past 8 rows (↑ ↓ or the wheel; "4–11 of 20" says where you are), and lists the common built-ins too: `/model` and `/effort` (then their values: `/model sonnet`), `/resume`, `/review`, `/init`, `/rewind`, `/usage` and more. A row says "opens a screen" when the command, sent alone, opens a screen in the terminal.

- Herdr app mod: a message you sent with pictures shows whole. Its "2 images" line was not counted in the row's height, so the bottom of the bubble was cut off.
- Herdr app mod: a code block fenced with `~~~`, or with more words after its language (`` ```js title=a.js ``), shows whole. Before, the first was cut off at the bottom, and the second drew the rest of the message as code.

### ⚙️ Changes

- Herdr app mod: after `/model ` the "/" list offers each agent's real models with their names: Claude Code's full ids (`claude-opus-5-5` "Opus 5.5", then the aliases), Codex's models, pi's models of the providers it has a key for (`provider/model`), and agy's. Type any part of a name ("sonnet", "flash") to find one. Codex's `/model` is back in the list.

- Herdr app mod: a "/" command that opens a menu or a screen in the terminal is not sent alone. `/model` stays in the box with a note: add a value, like `/model sonnet`. Commands that need the terminal and take no value (`/status`, `/config`, `/permissions` …) are no longer in the "/" list. The phone page refuses them the same way.

- Herdr app mod: the "working" spinner turns more smoothly: a short arc over a faint ring, one turn in 0.9 s, like Herdr's own.
