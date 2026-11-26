<!-- Each user-visible change adds one line here, in the same branch. At release this file becomes vX.Y.Z.md. -->
## What's New in vX.Y.Z

One short line: what this release is about.

### ✨ New
- The Mac app has the Since you left card and Summaries too, as in the Herdr app (Summaries is off by default).
- Herdr mod: a Resources window (⌘⇧R, or the new button in the top bar) lists the docs (in Markdown, Web pages, PDFs and Images), artifacts, PRs, one-line commands (a click copies one) and links the agent pointed you to in this conversation; a badge counts the new ones. It sits right of the chat, which does not move, and it is always as tall as the room above the chat input. Click a group's title to fold it; a long group shows 30 and "Show N more", and going back to an agent shows its list at once.
- Herdr mod: web addresses in the agent's replies (pages, GitHub PRs, issues and CI runs, claude.ai artifacts, localhost) are now links you can click.
- Herdr app sidecar on Windows: it reaches herdr-core on its named pipe, so it no longer answers "herdr-core is not running". There is no Windows release yet.

### 🔒 Security

### 🐛 Bug Fixes
- A chat no longer shows "No messages yet" when the end of the agent's log is filled with large lines that are not messages (a request record, a message with pictures). The Herdr mod, its glance, cards and summaries, and the Mac app now read further back until they find the last messages.
- Herdr app sidecar on Windows: Claude panes find their log, so Glance and the chat show them. It reads the start time and the program name as Claude Code writes them on Windows.

### ⚙️ Changes
- README: how to uninstall the Herdr app mod by hand, and what Summaries send and keep.
