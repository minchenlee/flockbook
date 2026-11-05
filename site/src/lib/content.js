// Links the owner fills in before launch. null renders a disabled placeholder.
export const links = {
	download: 'https://github.com/minchenlee/flockbook/releases/tag/v0.8.0', // the last Mac app release (releases ship the mod only); assets: Flockbook-<version>.dmg / .zip
	github: 'https://github.com/minchenlee/flockbook'
};

// Each feature names a still stem in /media/: <image>-light.png and, optionally, <image>-dark.png.
// A feature with no image (or no light still on disk) is left out of the carousel.
// `body` is one sentence: it is the slide caption.
export const features = [
	{
		id: 'read',
		title: 'Read what each agent did',
		body: 'The agent’s own session log, shown as a document with Markdown, code, tables and tool calls folded into short summaries.',
		image: 'conversation',
		alt: 'A conversation with an agent, shown as formatted text with folded tool calls.'
	},
	{
		id: 'needs-you',
		title: 'See which one needs you',
		body: 'Agents that wait for you go to the Needs you list, and macOS notifies you when a tab becomes blocked.',
		image: 'needs-you',
		alt: 'The Needs you list with two agents waiting for an answer.'
	},
	{
		id: 'reply',
		title: 'Reply in one place',
		body: 'Type under the conversation and press Return to send, with input methods, dictation and pasted images.',
		image: 'reply',
		alt: 'The reply field under a conversation, with a pasted image.'
	},
	{
		id: 'answer',
		title: 'Answer questions and approvals',
		body: 'Pick an option or choose Allow or Deny from a card, without opening the terminal.',
		image: 'question',
		alt: 'A question card with three options and a permission card with Allow and Deny.'
	},
	{
		id: 'pictures',
		title: 'Pictures in the conversation',
		body: 'Screenshots you paste and pictures the agent reads show in the conversation, on the Mac and on your phone.',
		image: 'pictures',
		alt: 'A conversation with a pasted screenshot of a cramped app screen and the agent’s screenshot of the fixed screen.'
	},
	{
		id: 'queued',
		title: 'See what is queued',
		body: 'Messages you send while an agent works stay above the reply field until the agent takes them, and you can pull Claude Code’s queue back to edit it.',
		image: 'queued',
		alt: 'Two queued messages above the reply field while the agent works, with Pull back to edit.'
	},
	{
		id: 'phone',
		title: 'Your agents on your phone',
		body: 'Read, reply, answer and approve from your phone over Tailscale or your local network. It is off until you turn it on.',
		image: 'phone',
		alt: 'Two phone screens: the list of agents that need you, and a conversation with pictures.'
	},
	{
		id: 'push',
		title: 'Notifications on your phone',
		body: 'Your phone shows a notification when an agent needs you, also when the page is closed.',
		image: 'push',
		alt: 'A phone lock screen with Flockbook notifications for agents that need you.'
	},
	{
		id: 'overview',
		title: 'Every workspace at a glance',
		body: 'Overview shows every tab of every workspace with its state: working, waiting for you, or done.',
		image: 'overview',
		alt: 'The Overview screen with workspaces and the state of each tab.'
	},
	{
		id: 'terminal',
		title: 'The terminal is one shortcut away',
		body: 'Press Control-backtick (⌃`) to see the real terminal of a pane.',
		image: 'terminal',
		alt: 'The terminal of an agent pane shown in a sheet over the conversation.'
	}
];

export const agents = ['Claude Code', 'Codex', 'Cursor', 'pi'];
