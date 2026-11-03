# Next release

## ✨ New

- Scroll to the top of a chat to load earlier messages, 2 MB of the log at a time, with a "Loading earlier messages" note while it reads.
- The sidebar's rows line up: icons and workspace letters share one column, and the status marks end where the + ends.
- ctrl-` and the Terminal button now close the terminal window when it is open.
- Every chat opens at its newest message: when you switch to it, switch back to it, or come back from Overview or Needs you.
- A Flockbook layout for herdr-app: a top bar, a hidden sidebar, and chat and "Needs you" screens (placeholders for now).
- The top bar shows one pill per agent in the space, with a spinner while it works and a check when it is done. Click a pill to read that agent. It also has the sidebar button (⌘B), "+" for a new agent, and "Needs you" with the number of agents that wait.
- The sidebar: New tab, Needs you with a badge, the workspaces with a status mark per tab (3 and "+N" past 5), and Settings.
- The "Needs you" screen lists the agents that wait in every space; pick one to see it and open its chat. "All clear" when none waits.
- The chat shows the agent's conversation. Your messages sit on the right; tool calls fold into a chip such as "Ran 3 commands" (click to see each call). It opens at the newest message and follows new ones; scroll up and "Jump to latest" brings you back. Long messages are cut, with "Show all".
- Under the chat: tags for the model, effort, context (red from 80%), cost, folder, branch and agent, and a "Reply to <agent>" box. Enter sends. Attach files with "+" or by dropping them on the chat. Each agent keeps its own unsent draft; a failed send puts your text back with the reason.
- When an agent asks to run something, the chat and "Needs you" show the command with "Allow once" and "Deny". When it asks a question, pick the options (or type your own) and press "Send answer". If the prompt changed before you pressed, the card shows the new one and nothing is sent.
- A tool call that is still running shows as "Running · <tool>" with a spinner, so it is not mistaken for an approval request.
- A black-and-white look: a dark theme and a light one that follow the system appearance, with no colour accents and clear borders.
- Each agent pill has a × that closes its tab.
- "Terminal" in the top bar opens the agent's own terminal in a window you can move; "Close" in its bar shuts it.
- Clicking a workspace in the sidebar shows its chat with its first agent, also from "Needs you".
- The chat only shows agents of the workspace you are in; with none, the top bar still has "+".
- Tags under the chat are shorter: the folder's last part, and nothing twice.
- When a conversation cannot be loaded, the chat says why in plain words.
- The top bar now looks like Flockbook's: one capsule with every tab of the workspace (agents and shells), a status ring or spinner before each name, and × on the shown tab; a second capsule holds "+" and the terminal button. A shell tab shows its terminal in place of the chat.
- Flockbook's shortcuts: ⌘T new tab, ⌘W close, ⌘1–9 pick a tab, ⇧⌘[ / ⇧⌘] and ⌃Tab previous and next tab, ⌘⌥↑ / ⌘⌥↓ previous and next workspace, ⌃⇧⌘← / → move the tab, ⌃` the agent's terminal, ⇧⌘I Needs you, ⇧⌘N new workspace, ⌘B sidebar, ⌘= ⌘- ⌘0 text size. ⌘⌥↑ / ⌘⌥↓ also work while you type in the chat.
- Long messages show in full in the chat; there is no "Show all" any more.
- The top bar has the chat's background and no "Needs you" (it stays in the sidebar). The sidebar button no longer jumps: while the sidebar is open it sits in the sidebar's header.
- Overview (sidebar, or ⇧⌘\): every tab of every workspace as a card with its status, workspace, agent and how long ago it changed, grouped by status (Needs you, Working, Done and resting) or by workspace. Click a card to open that tab.
- A "Flockbook Mac" theme in dark and light, with the macOS app's greys and its green, blue and orange status colours. Pick it in the theme menu.
- Smoother motion: the selected tab's fill fades between tabs, another agent's chat and a new approval or question card fade in (the card also rises into place), and spinners turn faster.
- "Jump to latest" is now a round ↓ button that fades in while you read older messages.

## 🔒 Security

## 🐛 Bug Fixes

## ⚙️ Changes
