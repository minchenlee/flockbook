<!-- Each user-visible change adds one line here, in the same branch. At release this file becomes vX.Y.Z.md. -->
## What's New in vX.Y.Z

One short line: what this release is about.

### ✨ New

- Herdr app mod: Flockbook settings have a Phone page switch. On runs the phone page in the background as a user service (it keeps running after Herdr app quits), names your Tailscale login, and shows the one `tailscale serve` command, then the address and a QR code. Flockbook never changes Tailscale settings. *(preview)*

- Phone page on the Rust helper (`flockbook-modfeed serve`): scroll up for older messages, as on the Mac hub. New rows still come while you read them.
- Phone page on the Rust helper: spaces and tabs as the Herdr app shows them, also tabs on other machines. Before, every terminal was in one tab. Reading, approving and sending on another machine go through the helper there.
- Phone page on the Rust helper: messages you typed while the agent works show at the end of the conversation, and Claude Code's can be pulled back to edit, as on the Mac hub.
- Phone page on the Rust helper: images you add in the composer go out with your message. PNG, JPEG and WebP up to 10 MB, without their metadata (a photo's location is removed); HEIC is refused with a plain message.
- Phone page on the Rust helper: notifications when an agent needs you or is done, with the page closed too. They hold no conversation text and are encrypted for your phone.
- Phone page on the Rust helper: pictures in the conversation show, on this machine's terminals. PNG, JPEG and WebP up to 10 MB are sent as they are in the log.

### 🔒 Security

### 🐛 Bug Fixes

- Herdr app mod: the QR code on the Phone page was blank (a white box). It shows now, and a new address gets a new code.
- Herdr app mod: with the Phone page On, the one `tailscale serve` command shows right under Running, and the address and QR code appear once it has run, without reopening settings. When port 443 already goes to another service, the command uses 8443 instead of replacing it, and says so.
- Herdr app mod: a chat whose newest part is mostly long tool output (screenshots) loads earlier messages by itself until the screen is full. Before, it showed only the last reply and scrolling up loaded nothing.
- Herdr app mod: a chat reply that mixes Chinese with English words or numbers (often in a list) no longer loses its last line under the next row.
- Herdr app mod: an expanded "Ran N commands" shows a multi-line command (a heredoc) as its first line with "…", instead of drawing every line over the rows around it.
- Herdr app mod: text in a code block can be selected and copied with ⌘C, also across lines, like the rest of a reply. Right-click a code block for Copy code, which copies the whole block.

### ⚙️ Changes

- Herdr app mod: Flockbook settings are laid out as Herdr's own Settings: General, Phone page and Updates in a sidebar on the left, each setting with its line on the left and its switch on the right.
