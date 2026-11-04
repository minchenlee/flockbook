<!-- Each user-visible change adds one line here, in the same branch. At release this file becomes vX.Y.Z.md. -->
## What's New in vX.Y.Z

One short line: what this release is about.

### ✨ New
- Settings > General > Backend chooses what Flockbook follows: Automatic (default: whichever of the herdr TUI and herdr-core answers, with no warning about the other), herdr-core only, Herdr TUI only, or Both. A new choice applies at once.

- A Rust version of the Herdr app mod's sidecar (`xplat/flockbook-modfeed`, on herdr-core), so the mod can run on Linux too (preview; Windows to follow).
- Herdr app mod (preview): tabs on another machine (`hc machine`) get the chat too. The Rust helper runs on that machine (Linux x86_64/aarch64 or macOS; not Windows yet). Before it puts the helper there, a card shows the machine, the path, the size and the SHA-256, and installs only when you press Install; the tab menu can remove it. Tested on a live machine: the chat feed, install, send and remove. Approve and answer on a remote tab are tested only against a simulated machine so far. The Swift helper still skips these tabs.
- The Herdr app mod has an install script, `install.sh`, for your real herdr-app home: it never overwrites your files, merges keys, and runs `herdr-app check` (`--dry-run` to preview). A missing helper now says where to get it.

### 🔒 Security

### 🐛 Bug Fixes
- Two presses of Allow at the same moment (two views, or a double key) could each send a key; now only one does.
- When two presses of Allow meet, the one that loses now says "Press again" instead of "already answered", and a run that was killed mid-press no longer blocks the next press.

### ⚙️ Changes
- The Herdr app mod source moved from `herdr-mod/` to `mod/`. The release zip name does not change.
