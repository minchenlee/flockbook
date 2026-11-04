<!-- Each user-visible change adds one line here, in the same branch. At release this file becomes vX.Y.Z.md. -->
## What's New in vX.Y.Z

One short line: what this release is about.

### ✨ New
- Settings > General > Backend chooses what Flockbook follows: Automatic (default: whichever of the herdr TUI and herdr-core answers, with no warning about the other), herdr-core only, Herdr TUI only, or Both. A new choice applies at once.

- `flockbook-modfeed serve` shows the phone page from the Rust helper on macOS and Linux, without the Mac app: off unless started, 127.0.0.1 only, reached through Tailscale Serve for your own account only (guide: `xplat/flockbook-modfeed/SERVE.md`).
- A Rust version of the Herdr app mod's sidecar (`xplat/flockbook-modfeed`, on herdr-core), so the mod can run on Linux too (preview; Windows to follow).
- Herdr app mod (preview): tabs on another machine (`hc machine`) get the chat too. The Rust helper runs on that machine (Linux x86_64/aarch64 or macOS; not Windows yet). Before it puts the helper there, a card shows the machine, the path, the size and the SHA-256, and installs only when you press Install; the tab menu can remove it. Tested on a live machine: the chat feed, install, send and remove. Approve and answer on a remote tab are tested only against a simulated machine so far. The Swift helper still skips these tabs.
- The Herdr app mod has an install script, `install.sh`, for your real herdr-app home: it never overwrites your files, merges keys, and runs `herdr-app check` (`--dry-run` to preview). A missing helper now says where to get it.
- Install the Herdr app mod in one line on macOS or Linux: `curl -fsSL https://github.com/minchenlee/flockbook/releases/latest/download/install.sh | sh`. It checks the helper against the release's SHA256SUMS before it runs anything. The install itself is now `flockbook-modfeed install`, so python3 is no longer needed.
- The mod's zip carries the helper for macOS (universal) and Linux (x86_64 and arm64, static), and the install keeps them all in `<herdr-app home>/flockbook/remote/` for your other machines. On Linux the + file dialog is off; drop files or paste an image.

### 🔒 Security

### 🐛 Bug Fixes
- Two presses of Allow at the same moment (two views, or a double key) could each send a key; now only one does.
- When two presses of Allow meet, the one that loses now says "Press again" instead of "already answered", and a run that was killed mid-press no longer blocks the next press.

### ⚙️ Changes
- The Herdr app mod source moved from `herdr-mod/` to `herdr-app/`. The release zip name and its contents do not change.
- The mod's page on the Herdr layout gallery moves with the folder: it is now https://herdr.dev/layouts/minchenlee/flockbook--herdr-app/ . The old address (`flockbook--mod`) stops working.
