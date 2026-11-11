<!-- Each user-visible change adds one line here, in the same branch. At release this file becomes vX.Y.Z.md. -->
## What's New in vX.Y.Z

One short line: what this release is about.

### ✨ New


### 🔒 Security

### 🐛 Bug Fixes
- Chat: a message sent right after `/clear` no longer says "Another agent is in this pane now". It says the agent started a new conversation, and the chat shows the new one at once.
- Answers to Claude Code questions go through again on the hc backend: long questions (drawn with a "│" bar since Claude Code 2.1.322), an "edit in <your editor>" key line, and typed answers with capitals. An answer hc can't type is refused before any key, with a note to answer in the terminal.

### ⚙️ Changes
