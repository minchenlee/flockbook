<!-- Each user-visible change adds one line here, in the same branch. At release this file becomes vX.Y.Z.md. -->
## What's New in vX.Y.Z

One short line: what this release is about.

### ✨ New


### 🔒 Security

### 🐛 Bug Fixes
- Chat (Herdr app, Claude Code): a dim "/" prompt suggestion in an empty box no longer stops a send with "The agent's box has text in it". The sidecar looks once at the screen's attributes: dim text with the cursor at the box start is a suggestion. Text you typed still stops the send. Tested on the dim empty-box placeholder; a real "/" suggestion was not seen in testing, and if one is not dim the send is refused as before.
- Question cards: on a busy machine, an answer no longer stops with "Another agent is in this pane now" while the same agent is there. Each key waits longer for the question to show, and a real screen change says so, with "check the terminal" when some keys already went in.
- Chat: a message sent right after `/clear` no longer says "Another agent is in this pane now". It says the agent started a new conversation, and the chat shows the new one at once.
- Answers to Claude Code questions go through again on the hc backend: long questions (drawn with a "│" bar since Claude Code 2.1.322), an "edit in <your editor>" key line, and typed answers with capitals. An answer hc can't type is refused before any key, with a note to answer in the terminal.
- Question cards: a comma in an option name of a single-choice question (such as "Yes, apply it") no longer sends the answer to the terminal. Only multi-select questions refuse a comma, because the review screen joins picks with commas. Thanks to @ianjamila (#14).
- Question cards (Herdr app, Claude Code): when the terminal's review screen could not show your answers (an option with a comma or a ● or → mark, two options alike, or such an answer you typed), the card says so before you answer. Send is off and "Answer in terminal" opens the pane. A typed answer is checked again as you type. Before, the card refused only after you pressed Send.
- Herdr app: the Terminal button and ctrl-` work again on a tab whose agent stopped reporting its status (Claude Code can stop after it moves to a new session id; hc then lists it as zsh). They open the tab's focused terminal, else its first one.
- Mac app question cards (Claude Code): a comma in an option of a single-choice question no longer sends the answer to the terminal, as in the Herdr app. When the terminal's review screen could not show your answers, the card says so before you answer: Send is off, "Answer in terminal" opens the pane, and a typed answer is checked again as you type.

### ⚙️ Changes
- The Swift `flockbook-modfeed` sidecar is retired. The Rust helper, which releases already ship, is the only one; the CI parity check between the two is gone.
