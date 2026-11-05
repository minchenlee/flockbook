<!-- Each user-visible change adds one line here, in the same branch. At release this file becomes vX.Y.Z.md. -->
## What's New in vX.Y.Z

One short line: what this release is about.

### ✨ New

- Herdr app: drag a tab in the top bar to change the tab order. A line shows where it goes; dragging out of the bar cancels.
- Herdr app: drag a workspace in the sidebar up or down to change the workspace order. A line shows where it goes; dragging out of the list cancels.
- Herdr app mod: a Flockbook settings screen (check for updates, status row, helper on other machines), a quiet line when a newer Flockbook is out, and the install upgrades an older, unchanged Flockbook (a gallery copy) in place. *(preview)*

- Phone page on the Rust helper (`flockbook-modfeed serve`): scroll up for older messages, as on the Mac hub. New rows still come while you read them.
- Phone page on the Rust helper: spaces and tabs as the Herdr app shows them, also tabs on other machines. Before, every terminal was in one tab. Reading, approving and sending on another machine go through the helper there.
- Phone page on the Rust helper: messages you typed while the agent works show at the end of the conversation, and Claude Code's can be pulled back to edit, as on the Mac hub.
- Phone page on the Rust helper: images you add in the composer go out with your message. PNG, JPEG and WebP up to 10 MB; HEIC is refused with a plain message.
- Phone page on the Rust helper: pictures in the conversation show, on this machine's terminals. PNG, JPEG and WebP up to 10 MB are sent as they are in the log.

### 🔒 Security

### 🐛 Bug Fixes

- Herdr app mod: ⌘V in the reply box attaches copied files and images. A file copied in Finder is attached instead of its name typed; a screenshot or a browser's Copy Image (also TIFF only) is attached as a PNG. Text still pastes as text, also from Excel, Numbers or Word.
- Herdr app mod: Enter in the middle of the reply sends it without a line break at the cursor. Before, that line break went out, and came back with the text after a failed send.
- Herdr app mod: Shift+Enter in the reply box starts a new line at the cursor. Before, it sent the reply. Enter still sends.
- Herdr app mod: a chat reply in Chinese or Japanese, or with inline code, no longer loses its last line under the time row. Row heights now use the size the text is drawn at (16 px, 26 px lines) and keep whole words and code together.
- Herdr app mod: on Herdr 0.1.13 you can select and copy text in the chat again. Before, a drag in a reply selected nothing.

### ⚙️ Changes
