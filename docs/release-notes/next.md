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
- Question cards (Herdr app, Claude Code): answers to questions with a comma, ● or → in an option now go through. Before Submit, the card checks that the terminal's review screen shows your answers word for word. It stops only when two different answers would look the same there: it says so as you pick or type, Send is off, and "Answer in terminal" opens the pane. Before, such a card could stop on the review screen without a word; now it stays and tells you why.
- Herdr app: the Terminal button and ctrl-` work again on a tab whose agent stopped reporting its status (Claude Code can stop after it moves to a new session id; hc then lists it as zsh). They open the tab's focused terminal, else its first one.
- Mac app question cards (Claude Code): answers to questions with a comma, ● or → in an option now go through, as in the Herdr app. Before Submit, the card checks that the terminal's review screen shows your answers word for word. It stops only when two different answers would look the same there: it says so as you pick or type, Send is off, and "Answer in terminal" opens the pane. If sending fails after moving to the review screen, the card stays read-only and tells you why.
- Agents started through wrapper commands (such as `secretspec run -- claude` or `codex`) now show their conversation and can be answered from Flockbook.
- Question cards (Claude Code, Herdr app and Mac app): when an option is too long for one line on the screen, the card now says so before Send and asks you to answer in the terminal, instead of stopping on the review screen. A long question with a line that starts with "→" no longer stops there either.

### ⚙️ Changes
- The Swift `flockbook-modfeed` sidecar is retired. The Rust helper, which releases already ship, is the only one; the CI parity check between the two is gone.
