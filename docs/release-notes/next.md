<!-- Each user-visible change adds one line here, in the same branch. At release this file becomes vX.Y.Z.md. -->
## What's New in vX.Y.Z

One short line: what this release is about.

### ✨ New

- Herdr app: drag a tab in the top bar to change the tab order. A line shows where it goes; dragging out of the bar cancels.
- Herdr app: drag a workspace in the sidebar up or down to change the workspace order. A line shows where it goes; dragging out of the list cancels.
- Herdr app mod: Flockbook settings in a dialog over the window, like Herdr's own Settings (check for updates, status row, helper on other machines), a quiet line when a newer Flockbook is out, and the install upgrades an older, unchanged Flockbook (a gallery copy) in place. *(preview)*
- Herdr app mod: Flockbook settings have a Phone page switch. On runs the phone page in the background as a user service (it keeps running after Herdr app quits), names your Tailscale login, and shows the one `tailscale serve` command, then the address and a QR code. Flockbook never changes Tailscale settings. *(preview)*

### 🔒 Security

### 🐛 Bug Fixes

- Herdr app mod: ⌘V in the reply box attaches copied files and images. A file copied in Finder is attached instead of its name typed; a screenshot or a browser's Copy Image (also TIFF only) is attached as a PNG. Text still pastes as text, also from Excel, Numbers or Word.
- Herdr app mod: Enter in the middle of the reply sends it without a line break at the cursor. Before, that line break went out, and came back with the text after a failed send.
- Herdr app mod: Shift+Enter in the reply box starts a new line at the cursor. Before, it sent the reply. Enter still sends.
- Herdr app mod: a chat reply in Chinese or Japanese, or with inline code, no longer loses its last line under the time row. Row heights now use the size the text is drawn at (16 px, 26 px lines) and keep whole words and code together.
- Herdr app mod: on Herdr 0.1.13 you can select and copy text in the chat again. Before, a drag in a reply selected nothing.

### ⚙️ Changes
