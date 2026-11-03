<!-- Each user-visible change adds one line here, in the same branch. At release this file becomes vX.Y.Z.md. -->
## What's New in vX.Y.Z

One short line: what this release is about.

### ✨ New
- herdr-core: terminals of herdr-core (the official Herdr app's core) show beside the herdr server's, grouped by the app's spaces and tabs, with their conversations, approval and question cards, layout changes, other machines and scrollback.
- Antigravity CLI (agy): agy panes show their conversation from the agy transcript, with tool calls and their results, and its command prompts get the approval card and notification buttons, and its single-choice questions the question card.

### 🔒 Security

### 🐛 Bug Fixes

- Claude Code: words that Claude Code logs as thinking right before a tool call now show in the conversation, as in the terminal.
- Claude Code: a tab whose conversation went on in a new log (`continued-in`) shows the new log, instead of "Waiting for the lead's log" or a conversation that stopped.

### ⚙️ Changes

- Phone app: a black and white frosted-glass look with no blue (orange, green and red still mean needs you, done and error). The bar floats as a capsule, and opening an agent turns its row into the bar.
