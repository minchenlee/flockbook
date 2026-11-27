<!-- Each user-visible change adds one line here, in the same branch. At release this file becomes vX.Y.Z.md. -->
## What's New in vX.Y.Z

One short line: what this release is about.

### ✨ New
- The Mac app has the Since you left card and Summaries too, as in the Herdr app (Summaries is off by default).
- Herdr mod: a Resources window (⌘⇧R, or the new button in the top bar) lists the docs (in Markdown, Web pages, PDFs and Images), artifacts, PRs, one-line commands (a click copies one) and links the agent pointed you to in this conversation; a badge counts the new ones. It sits right of the chat, which does not move, and it is always as tall as the room above the chat input. Click a group's title to fold it; a long group shows 30 and "Show N more", and going back to an agent shows its list at once. A long conversation shows its newest resources first and fills in the older ones while it says "Loading earlier… N%".
- Mac app: a Resources panel (⌘⇧R, or the new toolbar button with a badge of new items) lists the same docs, artifacts, PRs, commands and links as the Herdr mod, right of the chat, which does not move; Show in chat jumps to the message. It has the sidebar's look. A long Claude Code or Codex conversation is read newest first: the newest items show at once, and "Loading earlier… N%" shows under the list while the older parts are read. It says "Reading this chat…" until it finds an item or reads the whole log (or "Can't find this chat's log"); later reads parse only what the log grew by. It also lists the files an Antigravity CLI (agy) agent links as `file://` addresses.
- Herdr mod: web addresses in the agent's replies (pages, GitHub PRs, issues and CI runs, claude.ai artifacts, localhost) are now links you can click.
- Herdr app sidecar on Windows: it reaches herdr-core on its named pipe, so it no longer answers "herdr-core is not running". There is no Windows release yet.

### 🔒 Security

### 🐛 Bug Fixes
- A chat no longer shows "No messages yet" when the end of the agent's log is filled with large lines that are not messages (a request record, a message with pictures). The Herdr mod, its glance, cards and summaries, and the Mac app now read further back until they find the last messages.
- Herdr app sidecar on Windows: Claude panes find their log, so Glance and the chat show them. It reads the start time and the program name as Claude Code writes them on Windows.
- Herdr mod: Resources lists the files an Antigravity CLI (agy) agent links in its replies (`file://` links), and finds its relative paths by the files its tools named.

### ⚙️ Changes
- herdr-app/README.md is now a short landing page: what Flockbook is, a screenshot, the one-line install and the main features. The developer notes are below "For developers". The root README shows the new features and screenshots too.
- README: how to uninstall the Herdr app mod by hand, and what Summaries send and keep.
