<!-- Each user-visible change adds one line here, in the same branch. At release this file becomes vX.Y.Z.md. -->
## What's New in vX.Y.Z

One short line: what this release is about.

### ✨ New

- Herdr app mod: right-click a message in the chat and choose "Copy message" to copy its whole text as Markdown. It works when selecting text and ⌘C copy nothing (a drag across a code box, or a reply that is still being written).

### 🔒 Security

### 🐛 Bug Fixes

- Herdr app mod: a message you sent with pictures shows whole. Its "2 images" line was not counted in the row's height, so the bottom of the bubble was cut off.
- Herdr app mod: a code block fenced with `~~~`, or with more words after its language (`` ```js title=a.js ``), shows whole. Before, the first was cut off at the bottom, and the second drew the rest of the message as code.

### ⚙️ Changes
