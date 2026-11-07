<!-- Each user-visible change adds one line here, in the same branch. At release this file becomes vX.Y.Z.md. -->
## What's New in vX.Y.Z

One short line: what this release is about.

### ✨ New

### 🔒 Security

### 🐛 Bug Fixes

- Herdr app mod: a Codex chat stays filled once the agent runs a command under automatic approval review or starts a subagent. Codex writes a session file for each of its subagents in the same folder, and the helper took it for a second conversation and showed none.
- Herdr app mod: a note another agent sends with `hc send` shows as one line, "↳ docs-worker: first line", that opens to the whole note on a click, in the chat and on the Phone page. Before, it was a full message bubble with its `<hc>` tag, or it did not show at all (in Codex chats, and short ones in Claude Code chats).

### ⚙️ Changes
