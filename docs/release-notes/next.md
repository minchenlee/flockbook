<!-- Each user-visible change adds one line here, in the same branch. At release this file becomes vX.Y.Z.md. -->
## What's New in vX.Y.Z

One short line: what this release is about.

### ✨ New
- Herdr app mod (herdr-mod/): a Flockbook layout, mod, keys and themes for the Herdr app, with tab capsules, chat with cards and older messages on scroll-up, Overview, and the flockbook-modfeed sidecar.
- Herdr app mod: question cards show one question at a time with Back and Next (a pick moves on), take 1-9 to pick and Enter to go on or send, and "Show all N questions" reads a picker's other questions so you can answer them all from the chat.
- Herdr app mod: older messages load only when you scroll up past the first one, not while you read.
- Herdr app mod: ⌘V with an image on the clipboard attaches it to the reply.
- Herdr app mod: move the pointer to the window's left edge to peek the sidebar; it closes when the pointer leaves.
- Herdr app mod: the top bar floats over the chat, Overview and Needs you; only its capsules have a background.
- Herdr app mod: right-click a tab, a workspace or an Overview card for its actions (rename, move, split, put away, close).
- Herdr app mod: a split tab shows its panes; the bar's panes button switches between the chat and the panes. The bar's buttons say what they do on hover.
- herdr-core: terminals of herdr-core (the official Herdr app's core) show beside the herdr server's, grouped by the app's spaces and tabs, with their conversations, approval and question cards, layout changes, other machines and scrollback.
- Antigravity CLI (agy): agy panes show their conversation from the agy transcript, with tool calls and their results, and its command prompts get the approval card and notification buttons, and its single-choice questions the question card.

### 🔒 Security

### 🐛 Bug Fixes

- Claude Code: words that Claude Code logs as thinking right before a tool call now show in the conversation, as in the terminal.
- Claude Code: a tab whose conversation went on in a new log (`continued-in`) shows the new log, instead of "Waiting for the lead's log" or a conversation that stopped.

### ⚙️ Changes

- Phone app: a black and white frosted-glass look with no blue (orange, green and red still mean needs you, done and error). The bar floats as a capsule, and opening an agent turns its row into the bar.
- Herdr app mod: the pointer at the window's left edge now slides the sidebar out as a floating panel over the content, so the content does not move.
- Herdr app mod: a question card has no outer border and puts Next and Send answer at the bottom right; after "Show all" in one view, the other views show every question too.
- Herdr app mod: the top bar's panes button is gone (a split tab shows its panes by itself; the tab's right-click menu still switches); Needs you starts its detail at the top.
- Herdr app mod: the floating sidebar panel is as tall as the window and slides in from the left.
- Herdr app mod: the "Or type your own answer" box in a question card takes clicks and typing again, looks like the options, and its number key moves the keyboard into it.
- Herdr app mod: the agent's terminal window has a cleaner title bar (name, "Terminal", a round close button) and rounder corners.
- Herdr app mod: "Loading earlier messages" shows under the top bar's tabs instead of behind them.
