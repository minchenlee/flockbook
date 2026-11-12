# Mod ↔ sidecar protocol (version 1)

The Flockbook mod draws the screens. A sidecar program reads the agents' session logs and talks to
herdr-core. This file is the contract between them. Any sidecar that prints these lines works with
the mod, whatever language it is written in (the one in the Flockbook repo is Rust,
`xplat/flockbook-modfeed`).

## Rules

- The mod runs one command per call with herdr-app's `exec(path, args)`. The sidecar answers and exits.
  `exec` allows 30 s and 8 MiB of output.
- The answer is **JSON Lines** on stdout: one JSON object per line, UTF-8, `\n` after each line.
- Every object has a `type`. The mod ignores types and fields it does not know.
- Line 1 is always `header`. The last line is always `end`.
- A known failure is an `error` line, then `end` with `"ok": false`. The exit code is 0.
  A non-zero exit code, or output without `header` and `end`, means the sidecar itself broke.
- The sidecar picks herdr-core the way the app does: `HC_SOCKET`, else `HC_SESSION`
  (the default session when unset). It inherits these from herdr-app's environment.
- Times are milliseconds since 1970 (`ts`). Text fields are cut at 20 000 characters, with a trailing `…`.
- The sidecar keeps no process between calls. It may keep a cache file (`--cache`).

## Finding the sidecar

1. The mod looks for `<herdr-app home>/flockbook/bin/flockbook-modfeed` (`appHome()` in the mod).
2. It runs `version`. It uses the sidecar only when `header.protocol` is `1`.
3. Missing sidecar, or another protocol number: the chat view says so and shows no conversation.
   The tab bar, sidebar and "Needs you" screen still work (they read herdr-app's own data).

**Reserved for a later release:** the mod folder will carry `sidecar.json`:

```json
{ "protocol": 1, "version": "0.1.0",
  "assets": [ { "os": "macos", "arch": "arm64", "url": "https://…/flockbook-modfeed-macos-arm64", "sha256": "…" } ] }
```

On first start, with no sidecar in `flockbook/bin/`, the mod downloads the asset for this machine
into `flockbook/bin/.download`, checks its SHA-256, makes it executable, then renames it to
`flockbook-modfeed`. A wrong checksum deletes the download and shows an error. Version 1 of the mod
does not download; `install-isolated.sh` copies the sidecar in.

## Commands

| Command | Arguments | Lines after `header` |
|---|---|---|
| `version` | — | `version` |
| `feed` | `--terminal <addr>` `[--cursor <c>]` `[--cache <file>]` `[--pictures]` | `log`, then `unchanged`, or `splice` and `row`s |
| `status` | `--terminal <addr>` `[--cache <file>]` | `status` |
| `card` | `--terminal <addr>` `[--answers <json>]` `[--cache <file>]` | `card` |
| `approve` | `--terminal <addr> --request <id> --allow\|--deny` `[--cache <file>]` | `done` |
| `answer` | `--terminal <addr> --request <id> --answers <json>` `[--cache <file>]` | `done` |
| `send` | `--terminal <addr> --text <s>` `[--file <path>]…` `[--expect-kind <agent>]` `[--expect-path <log>]` `[--cache <file>]` | `done` (`changed` when the terminal now runs another agent or conversation than the expected one; nothing is sent. Its message says "new conversation" when the same agent process wrote the expected log, e.g. after `/clear`. With `--cache` the log is looked up again and written to the cache, so the next `feed` reads it) |
| `queue` | `--terminal <addr>` `[--cache <file>]` (Rust sidecar only; `serve` and other machines) | `queued` |
| `commands` | `--terminal <addr>` `[--cache <file>]` (Rust sidecar only) | `commands` |
| `helper install` | `--machine <m> --sha256 <hex> [--cache <file>]` | `installed` |
| `helper remove` | `--machine <m> [--cache <file>]` | `removed` |
| `update-check` | `--home <herdr-app home>` `[--dismiss <x.y.z>]` (Rust sidecar only) | `update` |
| `phone status` / `on` / `off` | `--home <herdr-app home>` (Rust sidecar only) | `phone` |

`card` and `commands` only read. `approve` and `answer` send keys to the agent; the mod runs them only when the
owner presses a button, never on a timer. `send` types a message and presses Enter.

`<addr>` is the herdr-core terminal address (`t7`). The pane id form (`hc:t7`) also works.
`--cache` is a file the sidecar may read and write; the mod passes `<herdr-app home>/flockbook/cache.json`.
`--home <herdr-app home>`: every command takes it, and the mod always passes its `appHome()`, last; the last `--home`
counts, so a `--text` that says `--home` is not taken for it. The sidecar reads the
settings in `<home>/flockbook/config.json` from it (without it: the folder the sidecar is in, when that is
`<home>/flockbook/bin`). With `"remote_helper": "never"`, every command for a terminal on another machine, and
`helper install` and `remove`, stop with the error `remote-off` before anything runs there.

### Terminals on another machine (`m/t7`; Rust sidecar only)

The same helper runs on machine `m` with the same command and `--terminal t7`. First the sidecar
finds the platform there with `uname -sm` (`argv`). A machine without a helper target (Windows,
another CPU) gets `remote_helper_unsupported` and nothing else runs there. On Unix, hc `machine.exec`
gets `argv` `["sh", "-c", <fixed script>, "sh", <command>, <values>…]`: the fixed script runs
`"$HOME"/.local/share/flockbook/helper/<version>/flockbook-modfeed` (so the cwd of exec does not
matter), and the values are only in `"$@"`, so the shell never reads them: `--text` reaches the helper
exactly as given.
Its lines come back unchanged, so `header`, `end` and everything between are the helper's there.
The helper there keeps its own cache (`"$HOME"/.cache/flockbook/cache.json` there); this computer's
`--cache` is not passed on. `send --file`: the file is first put beside the terminal there (`terminal.put`) and the
text names that copy. Only one level: `a/b/t7` is `bad_terminal`.

The helper there must be this same version (`protocol` and `sidecar` both). It is never put on
another machine without the user's yes:

1. A command for `m/t7` answers `remote_helper_missing` (or `remote_helper_outdated`) with
   `install`: `{"machine","target","path","bytes","sha256","source"}`, or `null` when this
   Flockbook carries no helper for that platform.
2. The mod shows a card with the machine, the path there, the size and the sha256. `source`
   `"dev"` means the file is from `FLOCKBOOK_REMOTE_HELPERS` (a test or local build), not from the
   installed Flockbook: the card says so.
3. Only when the user presses Install: `helper install --machine m --sha256 <the card's sha256>`.
   The sidecar reads the file again, checks it against `SHA256SUMS` and that sha256, and writes it
   there in parts of at most 6 MiB with this run's own tag. Then one command there joins the parts,
   computes the sha256, and only when it matches makes the file runnable and moves it in place.
   When it does not match, the file is removed.
4. `helper remove --machine m` removes `~/.local/share/flockbook/helper` there (every version).
5. With `--cache <file>` the sidecar keeps `remote.json` beside that file: each machine's platform
   and "no helper there" for 60 s, and the local file's sha256 while its path, size and mtime stay
   the same. `helper install` and `remove` reset "no helper there". The install reads and checks the
   whole file again in any case.

The file comes from `<herdr-app home>/flockbook/remote/<target>/flockbook-modfeed`, with
`SHA256SUMS` in `remote/` (`<sha256>  <target>/flockbook-modfeed`, a leading `./` is fine) or in
`remote/<target>/`. A file that names the helper twice, or a sum that is not 64 hex digits, gives
`checksum`.
Targets: `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `universal-apple-darwin`.

## Lines

### `header` (every answer, line 1)

```json
{"cmd":"feed","protocol":1,"sidecar":"0.1.0","type":"header"}
```

| Field | Meaning |
|---|---|
| `protocol` | This document's version. Changes the mod must know about raise it. |
| `sidecar` | The sidecar's own version. |
| `cmd` | The command it ran. |

### `end` (every answer, last line)

```json
{"ms":38,"ok":true,"type":"end"}
```

`ok` is false after an `error` line. `ms`: time the command took.

### `error`

```json
{"code":"no-terminal","message":"no terminal t9 in herdr-core","type":"error"}
```

| `code` | Meaning |
|---|---|
| `usage` | Unknown command or missing argument. |
| `no-core` | herdr-core is not running. |
| `core` | herdr-core did not answer. |
| `no-terminal` | No terminal with that address. |
| `not-agent` | The terminal runs no agent (a plain shell). |
| `locate` | Finding the session log failed. |
| `read` | The log could not be read. |
| `bad_terminal` | Not a terminal address (`a/b/t7`, `/t7`). |
| `remote_helper_missing` | Machine `m` has no helper yet. `install`: what the install card shows, or `null`. |
| `remote_helper_outdated` | The helper there is another version. `remote`: its `protocol` and `sidecar`; `install` as above. |
| `remote_helper_unsupported` | No helper for that machine's platform (Windows, or another CPU). `message` says which. |
| `no-tailscale` | `phone on`: Tailscale is not installed, not running or not signed in. Nothing was written. |
| `by-hand` | `phone on` / `off`: the service file was set up by hand (no Flockbook marker). It was not changed. |
| `service` | `phone on`: launchctl or systemctl failed (bootstrap is tried a few times while launchd is still busy), or `id -u` failed; `message` says which and its output. |
| `bad-value` | `phone on`: the login (config.json `phone_user`) has spaces or control characters, or HC_SESSION / HC_SOCKET has a control character. Nothing was written. |
| `write` | `phone on` / `off`: the service file could not be written or removed; `message` says why. No `.new` file is left. |
| `remote-off` | The person chose Never for the helper on other machines (config.json `remote_helper`). Nothing ran there. |
| `remote_failed` | hc could not run it there: unknown machine, no answer in 30 s, or too much output. |
| `checksum` | `helper install`: the file is not the one the user said yes to, or it arrived changed. Nothing was installed. |

### `version`

```json
{"commands":["version","feed","status","card","approve","answer","send","paste-image","helper","update-check","queue","phone","commands"],"release":"0.9.0","type":"version"}
```

`commands`: the commands this sidecar has. The mod hides actions whose command is missing.
`release`: the Flockbook release this helper was built for (`x.y.z`, what `update-check` compares with), or
`null` for a build from source. The release's package job checks that every helper it ships says the tag's version.

### `update` (`update-check`)

```json
{"command":"curl -fsSL https://github.com/minchenlee/flockbook/releases/latest/download/install.sh | sh","current":"0.8.0","latest":"0.9.0","state":"newer","type":"update"}
```

`state`: `newer` (show the line), `current`, `dismissed` (`--dismiss` hid that version), `off` (config.json
`check_updates: false`; nothing asked), `unknown` (the sidecar has no release version: built from source; nothing
asked), `no-home`, or `error` (no answer yet, for example offline). Only `newer` shows anything. It asks
`api.github.com` for the latest release at most once a day, and keeps the answer in `<home>/flockbook/update-check.json`.

### `phone` (`phone status`, `on`, `off`)

```json
{"port":8787,"qr":"<home>/flockbook/phone-qr-1a2b3c4d.svg","read_only":false,"serve":{"command":"tailscale serve --bg 8787","reason":null,"set":true,"url":"https://box.tail1.ts.net/"},"service":{"file":"/home/me/.config/systemd/user/flockbook-serve.service","kind":"systemd","message":null,"state":"running"},"tailscale":{"dns":"box.tail1.ts.net","login":"me@example.com"},"tailscale_error":null,"type":"phone","user":"me@example.com"}
```

The phone page (`serve`, SERVE.md) as a user service. Every answer has this line, also after an `error`.
`on` writes the service file that `serve --print` makes, with a marker line and the app's `HC_SESSION` /
`HC_SOCKET`, and loads it (launchd agent `~/Library/LaunchAgents/dev.flockbook.serve.plist`; systemd user unit
`~/.config/systemd/user/flockbook-serve.service`). Run again, it writes the file again with the current
settings and restarts it. `off` stops it and removes that file, only when it has the marker.

- `tailscale`: the login and machine name from `tailscale status --json`, or null with `tailscale_error`.
- `user`, `port`, `read_only`: config.json `phone_user` (empty: the Tailscale login), `phone_port` (8787),
  `phone_read_only` (false).
- `serve`: whether Tailscale Serve sends to that port (`tailscale serve status --json`; only read), its
  address, and the one command to set it. When 443 `/` already goes to another local service, the plain
  command would replace it: `command` is `tailscale serve --bg --https=8443 <port>` (the first free of 8443,
  10443, …) and `reason` says why (else null). The address keeps a port other than 443
  (`https://box.tail1.ts.net:8443/`). `qr`: an SVG QR code of the address, once it is set:
  `<home>/flockbook/phone-qr-<8 hex of its sha256>.svg`, a new name for a new address; the helper removes its older
  `phone-qr*.svg` files there. The mod draws it from the path under the app home (`svg()` reads from the app root).
- `service.state`: `off` (no file), `running`, `stopped`, `error` (`message` says where its log is), or
  `by-hand` (a file without the marker; never changed).

### `installed` (`helper install`), `removed` (`helper remove`)

```json
{"bytes":4371392,"machine":"box","parts":1,"path":"~/.local/share/flockbook/helper/0.1.0/flockbook-modfeed","sha256":"7ac5…","source":"release","type":"installed"}
{"machine":"box","path":"~/.local/share/flockbook/helper","type":"removed"}
```

### `files`, `image` (`paste-image --dir <folder>`)

```json
{"paths":["/home/you/Desktop/Shot 1.png","/home/you/notes.txt"],"type":"files"}
{"path":"/home/you/.config/herdr-app/flockbook/paste/paste-20261105-140000-A1B2C3.png","type":"image"}
```

What ⌘V in the reply box attaches. `files`: files copied in Finder or a file manager (file URLs), in order;
an `image` line with `path: null` follows it. Otherwise `image`: `path: null` when the clipboard holds plain text
(also beside an image, as Excel, Numbers and Word put it) or nothing, so the box types the text; else the
clipboard's image saved as a new PNG in the folder (PNG as it is; on macOS TIFF and other image types are turned
into PNG). Pastes older than a day in that folder are removed; other files never.

### `log` (`feed`)

```json
{"agent":"claude","cursor":"1.9c1d….0.48213.5be0…","model":"claude-opus-5-5","path":"/…/session.jsonl","size":48213,"start":0,"type":"log"}
```

| Field | Meaning |
|---|---|
| `agent` | The agent kind: `claude`, `codex`, `pi`, `omp`, `cursor`, `grok`, … |
| `path` | The session log, or `null` when none was found yet (then `splice` has `count` 0). |
| `start`, `size` | The byte range read: from `start` (a line start) to `size`. |
| `cursor` | Opaque. Pass it back as `--cursor` on the next `feed` for this terminal. |
| `model` | The model the log names last, when known. |

### `unchanged` (`feed`)

```json
{"type":"unchanged"}
```

The log did not change since `--cursor`. The mod keeps its rows. No `splice` follows.

### `splice` (`feed`)

```json
{"at":12,"count":15,"type":"splice"}
```

The mod drops its rows from index `at` on, then appends the `row` lines that follow (`count - at`
of them). `count` is the number of rows after the splice. Without a valid `--cursor`, `at` is 0
and every row follows.

The sidecar reads the same range again each time and compares. A tool result that arrives later,
a footer that moves to the turn's new last row, or an approval that resolves, all show as a splice
from the first changed row. When the range grows past 8 MB, the sidecar starts a new 2 MB tail
(`at` 0, a new `start`).

### `older` (`feed --before <start> --path <log>`)

```json
{"count":83,"path":"…","start":74885870,"type":"older"}
```

The rows of the 2 MB of the log that end at `start` (the tail's start, or the last `older` start),
whole lines only. The `row` lines follow. `start` 0: the log's first row is in it. The cache and
the cursor are not touched. A `moved` error: the agent now writes another log than `--path`.

### `row` (`feed`)

With `--pictures` (what `serve` asks for, also with `--before`), the log is parsed with each line's offset, and
`user` and `tools` rows with pictures get `pics`: one entry per picture, the owner's for a user row, the calls'
result pictures in call order for a tools row. `{"format":"png","width":3,"height":4,"offset":1290,"index":0,
"digest":"<16 hex digits>"}`: the line's file offset, the picture's place in that line, and the first 8 bytes of
the SHA-256 of its base64. Codex, Grok and Cursor row ids then come from line offsets, so they differ from the
rows without `--pictures`. The mod never passes it.

One per conversation row, in order. `id` is stable for the same log and `start`: the mod keys row
heights by it.

| `k` | Fields | Shown as |
|---|---|---|
| `user` | `text`, `ts`, `images` (count, when any) | A right-aligned bubble. |
| | `pics` (with `--pictures` only) | Where the pictures are in the log, for `serve`. |
| | `from` (a note from another agent) | One line, "↳ docs-worker: first line", that opens to the whole note. |
| `assistant` | `text` (markdown), `ts` | Full-width markdown. |
| `tools` | `chips`, `ts` | A chip: "Ran 3 commands", expands to the calls. |
| `perm` | `perm`, `text` (the title), `ts` | The approval card (when `perm.state` is `pending`). |
| `question` | `question`, `ts` | The question card. |
| `status` | `text` | A centered note ("Context compacted"). |
| `sep` | `text` ("14:05", "Yesterday 09:12", "Mon 2 Nov 18:30") | A time separator. |
| `footer` | `text` ("01:41 · Worked 36s · 3 tools") | The turn footer, under the turn's last row. |

A note from another agent is a message that is one `<hc from="t2 docs-worker">…</hc>`, as herdr-core's `hc send`
types it. Its `user` row has `from` (the sender as hc wrote it: the terminal, then the name when it has one; `""`
without one) and `text` without the tag. A mod that does not know `from` shows the text as a bubble.

```json
{"id":"u1","k":"user","text":"Fix the login bug","ts":1793670000000,"type":"row"}
{"from":"t2 docs-worker","id":"u2","k":"user","text":"Done: notes updated.","ts":1793670001000,"type":"row"}
{"id":"a1","k":"assistant","text":"Looking at it now.","ts":1793670005000,"type":"row"}
{"chips":[{"calls":[{"done":true,"error":false,"name":"Bash","output":"ok","summary":"swift test"}],"symbol":"terminal","text":"Ran a command"}],"id":"a1-t3","k":"tools","ts":1793670006000,"type":"row"}
{"id":"f-a2","k":"footer","text":"01:40 · Worked 36s · 1 tool","type":"row"}
```

`chips[]`: `symbol` (an SF Symbols name such as `terminal` or `doc.text`; the mod maps it to its own icon), `text` (the chip label), `calls[]`. Each call: `name` (the
tool), `summary` (command or path), `done` (a result arrived), `error`, `output` (the first 400
characters of the result, when done).

`perm`: `title`, `command`, `tool`, `toolUseId`, `state` (`pending` or `resolved`). A pending
`perm` row means the log ends with a call that has no result yet. Whether the agent waits for an
approval is the `card` command's job.

`question`: `toolUseId`, `agent`, `delivery` (`picker`: keys drive the agent's picker;
`message`: the answer is a normal message), `state` (`pending`, `answered`, `declined`),
`answers` (when answered), `questions[]` with `id`, `header`, `text`, `multi`, and `options[]`
(`label`, `detail`, `recommended`).

### `status` (`status`)

```json
{"agent":"claude","source":"statusline","tags":[{"kind":"model","text":"Opus 5.5","warn":false},{"kind":"effort","text":"high","warn":false},{"kind":"context","percent":37,"text":"ctx 37%","warn":false},{"kind":"cost","text":"$0.42 est.","warn":false}],"type":"status"}
```

| Field | Meaning |
|---|---|
| `source` | `statusline` (Flockbook's status line wrapper is on), `log` (only the model, from the log), `none`. |
| `tags[]` | `kind` (`model`, `effort`, `context`, `cost`, `own`), `text`, `percent` (context only), `warn` (context 80% or more). |

The mod adds the cwd, the branch and the agent name from herdr-app's own `panes()` data.

### `card` (`card`)

What the agent waits for now, or nothing. The mod polls it only for an agent that herdr-app shows
as blocked, and for a pending `question` row.

```json
{"command":"rm -rf build","kind":"approval","request":"toolu_01AB…","title":"Bash command","tool":"Bash","type":"card"}
{"kind":"question","question":{"agent":"claude","delivery":"picker","questions":[{"header":"Color","id":"Pick a color?","multi":false,"options":[{"detail":"warm","label":"Red","recommended":true},{"detail":"cool","label":"Blue","recommended":false}],"text":"Pick a color?"}],"state":"pending","toolUseId":"screen-…"},"request":"screen-…","type":"card","unread":0}
{"type":"card"}
```

| Field | Meaning |
|---|---|
| `kind` | `approval`, `question`, or missing (no card). |
| `request` | The id to pass to `approve` or `answer`. It stays the same while the same prompt is on screen. Approval: the prompt box on screen (its id covers the tool, the command and the options); the log's call id only when the log's call is that box. |
| `title`, `command`, `tool` | Approval: what it asks to run, from the box on screen (or the log's call when it is the box). |
| `answered` | Approval: `true` when this prompt was answered already (by any view) and is still drawn. The mod shows no buttons. |
| `question` | Question: the same object as in a `question` row. |
| `unread` | Question: how many more questions exist than the card shows (Claude shows one at a time). `answer` reads them itself. |
| `blocked` | Question: why `answer` would refuse before any key, because the terminal's review screen could not show the answers (a hooked Claude question set: an option with a comma or a `●`/`→` mark, two options alike, or such an own answer). `card --answers` (the `answer` format, the answers so far) also checks the own answers. Missing while `unread` > 0. The mod turns Send off and shows it with a button that opens the terminal. |

An approval card appears only when the screen shows the prompt ("Do you want …?" with "1. Yes").
A running tool is not an approval.

### `queued` (`queue`)

The agent's own queue: messages typed while it works that it has not taken yet. Read only; the screen is read only
while a Claude Code, Codex or Grok Build agent works. No queue: `agent` is `""` and `items` is empty.

```json
{"type":"queued","agent":"claude","items":["Also check the lint step.",null],"pullable":true,"pullsAll":true}
```

- `items`: oldest first; `null` is a queued message whose text neither the log nor the screen shows.
- `pullable`: the agent's input box is empty, so the queue can be pulled back now.
- `pullsAll`: Claude Code (Up takes back every queued message as one text).

### `commands` (`commands`)

The "/" commands of an agent's terminal, for the list above the reply box: Claude Code, Codex, pi and agy. Read from
files only; no key goes to the agent. Another agent: `commands` is empty (the mod shows no list).

```json
{"type":"commands","agent":"claude","commands":[{"name":"clear","description":"Start a new conversation (clears the history)","source":"builtin"},{"name":"deploy","description":"Deploy the app","source":"project"},{"name":"model","description":"Set the AI model for Claude Code","source":"builtin","hint":"<model>","args":[{"value":"claude-fable-5-2","label":"Fable 5.2"},{"value":"claude-opus-5-5","label":"Opus 5.5"},{"value":"opus","label":"The latest Opus"}],"screen":true}],"screens":["config","help","hooks","mcp","memory","permissions","plugin","rewind","status","usage"]}
```

- Sources per agent (`agent`), the first one with a name wins:
  - `claude`: built-ins (the ones people use most that the chat can send: `add-dir`, `clear`, `compact`, `context`,
    `effort`, `export`, `init`, `model`, `resume`, `review`); `.claude/commands/**/*.md` in the terminal's folder (`project`), then in the user's
    home (`user`); `.claude/skills/<name>/SKILL.md` there and in the home (`skill`; not when its front matter says
    `user-invocable: false`); installed plugins that are not turned off (`enabledPlugins`), their commands and
    skills as `<plugin>:<name>` (`plugin`).
  - `codex`: a short set of its built-ins only (`/model` with the models of its cache). Its skills are not "/"
    commands.
  - `pi`: its built-ins; prompt templates `~/.pi/agent/prompts/*.md` (top level; `user`; the project's load only
    after the project is trusted, so they are left out); skills as `skill:<name>` from the project's `.pi/skills`,
    `.agents/skills` from the terminal's folder up to the repository root, `~/.pi/agent/skills` and `~/.agents/skills`
    (none when its settings say `"enableSkillCommands": false`). Commands of pi's extensions are not in files: not
    listed.
  - `agy`: its built-ins (one row a command; its other names in the description); skills from `~/.gemini/skills`
    and `~/.gemini/antigravity/skills` (`skill`).
- `description`: the front matter's `description`, else the file's first line; at most 120 characters.
- Each when it says something: `hint`, what may follow the name (`<model>`, `[file]`; for a file, its front matter's
  `argument-hint`); built-ins only: `args`, the values the mod offers after `/<name> `, each `{"value", "label"}` (no
  `label` where the value says it all; the mod also takes plain strings). Fixed lists in the helper: Claude Code's and
  agy's `effort`, pi's `thinking`. `model`, per agent (models.rs), where its `/model <value>` switches with no screen:
  - `claude`: its current models' full ids, then the aliases (a table: its list is inside its program), plus ids in
    `~/.claude/settings.json` (`modelSettings`, `availableModels`) and `~/.claude.json`
    `additionalModelOptionsCache`. It also saves the model as the default for new sessions, as its picker does.
  - `codex`: `~/.codex/models_cache.json`, the models its picker lists, in its order (this session only). No cache:
    `/model` takes no value and is in `screens`.
  - `pi`: `provider/id` of the providers it has a key for (`auth.json`, or `apiKey` in `models.json`), from
    `models-store.json` and `models.json`; only those `enabledModels` names when it names some. Another value opens
    pi's picker.
  - `agy`: a table (agy reads its list from its server): its slugs, `gemini-4-argon-medium`; it also saves the
    default.
  A value list with no hint gets the hint `<model>`. The mod finds a value by any part of its value or label.
  `screen: true`, sent bare
  it opens a screen in the terminal (a picker or a dialog: the chat cannot show it), so it goes out only with a
  value (the mod's row says "needs a value").
- `screens`: the built-ins that open a screen and take no value (`/status`, `/config`). They can never be sent from
  the chat, so they are not in `commands`; their names stay the built-ins' (a file of the same name is not listed).
- A screen command sent bare (the text is exactly `/<name>`, spaces trimmed, no files) is not sent: the mod keeps the
  text and shows a note ("/model opens a menu in the terminal. Add a value, like /model opus."; for a name in
  `screens`: "/status opens a screen in the terminal. Use it there."), and `send` refuses it with `opens_screen`
  (for the phone page and older mods). `/model sonnet` goes out.
- Sorted by name; at most 500. A name has only letters, digits and `-_.:`.
- The mod sends the full name (`send --text "/deploy"`): Claude Code's own "/" menu takes the Enter for a part of a
  name and runs its first match, and pi sends a part of a name to the model as a message. A full name, with or
  without an argument, runs the command in all four (spike).

### `done` (`approve`, `answer`, `send`)

```json
{"result":"sent","type":"done"}
{"message":"the approval on screen is not the one shown","result":"changed","type":"done"}
```

| `result` | Meaning | The mod |
|---|---|---|
| `sent` | The keys or the text went out. | Clears the card or the composer. |
| `changed` | Another card, or none, is on screen now. Nothing was sent. | Reads `card` again. |
| `unconfirmed` | herdr-core did not answer in time. The text or the key may still arrive. | Keeps the composer empty (a retry could send it twice) and says so; an approval shows as `answered`. |
| `no-identity` | Flockbook cannot tell the agent from another program in that terminal (hc: no process, for example under a wrapper script). Nothing was sent; a retry does not help. | Keeps the text or the card, shows `message`. |
| `answered` | `approve` only: this prompt was answered less than 60 s ago (or its key may still arrive) and is still on screen. Nothing was sent. | Shows the note; no buttons. |
| `busy` | `approve` only: another press of a prompt in this terminal (another view, or a double key) was being handled at the same moment. Nothing was sent and nothing is decided. | Keeps the card and its buttons, shows `message` (press again). |
| `opens_screen` | `send` only: the text is a screen command alone (`/model`, `/status`), and its screen would open in the terminal. Nothing was sent. `message` says what to add. | Keeps the text, shows `message`. |
| `box_not_empty` | `send` only: the agent's own input box already has text (the text would go after it: "/" and "/status" make "//status"). Nothing was sent. `message`: "The agent's box has text in it. Clear it in the terminal first. Nothing was sent." | Keeps the text, shows `message`. |
| `failed` | It did not go out. `message` says why. | Keeps the text or the card, shows `message`. |

`send` reads the screen right before it types. Where it finds the agent's input box and the box has text, it sends
nothing (`box_not_empty`). The box per agent (queue.rs `box_has_text`): Claude Code, the rows after "❯" between the
last two rules (its placeholder `Try "…"` or "Press up to edit queued messages" counts as empty; with prompt
suggestions on, its default, a suggested prompt stands dim in the empty box and the screen's text has no dim, so only
a box that starts with "/" counts as text: `"promptSuggestionEnabled": false` in `~/.claude/settings.json` reads
any text); Codex, the last
"›" row ("Ask Codex to do anything" is empty); pi, the rows between the last two rules; agy, "> " between the last
two rules (a mode's note "… (shift+tab to cycle)" is empty); Grok Build, "│ ❯" inside the last "╭ … ╰" box. Another
agent, a screen it cannot read, or a screen where the box is not found (a dialog, a menu): it sends as before, so a
doubt never blocks a message (but then the text may still go after what is in the box).

`approve` reads the screen again right before the keys and sends only while the box on screen is
still `request` (the app's `respondIfOpen`). Claude and agy: `--allow` sends `1` (allow once),
`--deny` sends Esc. Grok: Enter on "Yes, proceed", or Down then Enter on "No, reject", each key
checked against the focus. One answer per prompt: the claim is a file in `answered/` next to the
cache file, made before the key. It names the prompt by its screen id and by its tool and command,
so a box re-wrapped by a resize is the same prompt. It goes only when a screen read that worked shows
no approval box or a box with another command (a failed read keeps it), or after 60 s. A claim whose
key is `unconfirmed` lasts while the agent stays blocked.

`answer --answers` is one object per question, in order:
`[{"picks":[1]}, {"picks":[0,2]}, {"picks":[],"other":"my own answer"}]`. `picks` are option
indexes from 0; `other` is free text.

`send --file` names a file that the text already mentions by its path. For a terminal on this
machine the text goes as it is.

## Changing this protocol

- Adding a field or a line type keeps version 1: the mod ignores what it does not know.
- Removing or renaming a field, or changing what one means, raises `protocol`. The mod and the
  sidecar then ship together.
