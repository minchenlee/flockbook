<!-- Each user-visible change adds one line here, in the same branch. At release this file becomes vX.Y.Z.md. -->
## What's New in vX.Y.Z

One short line: what this release is about.

### ✨ New
- Herdr app: back on an agent's chat after 5 minutes or more, a "Since you left" card says how long you were away, how many messages came, and what the log shows (for example "Edited 5 files · pytest passed after the last edit"). A button jumps to your last message. The card reads only the agent's log; no model runs for it. Agents on this machine only.
- Herdr app: Summaries (Flockbook settings, off by default). When on, a model adds two lines to the "Since you left" card: the state of the work, and what it needs from you, marked "Summary · written by a model, may be wrong". The line from the log stays below them. Only this card shows them. When any agent on this machine stops while a chat is open (not only the one on screen; back from the Overview the card shows first and the two lines come in when the model answers), your message that started its last turn, the end of what the agent wrote and the line from the log go to the model: by default the agent's own program (Claude Code, Codex or pi, so the text goes where that conversation already goes), or one program you choose for every agent, with a model you pick. The settings say how often a model ran.


### 🔒 Security

### 🐛 Bug Fixes

### ⚙️ Changes
