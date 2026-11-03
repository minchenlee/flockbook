# Mod ↔ sidecar protocol (version 1)

The Flockbook mod draws the screens. A sidecar program reads the agents' session logs and talks to
herdr-core. This file is the contract between them. Any sidecar that prints these lines works with
the mod, whatever language it is written in (the first one is Swift, `flockbook-modfeed` in the
Flockbook repo).

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
| `feed` | `--terminal <addr>` `[--cursor <c>]` `[--cache <file>]` | `log`, then `unchanged`, or `splice` and `row`s |
| `status` | `--terminal <addr>` `[--cache <file>]` | `status` |
| `card` | `--terminal <addr>` `[--cache <file>]` | `card` |
| `approve` | `--terminal <addr> --request <id> --allow\|--deny` `[--cache <file>]` | `done` |
| `answer` | `--terminal <addr> --request <id> --answers <json>` `[--cache <file>]` | `done` |
| `send` | `--terminal <addr> --text <s>` `[--file <path>]…` `[--expect-kind <agent>]` `[--expect-path <log>]` | `done` (`changed` when the terminal now runs another agent or conversation than the expected one) |

`card` only reads. `approve` and `answer` send keys to the agent; the mod runs them only when the
owner presses a button, never on a timer. `send` types a message and presses Enter.

`<addr>` is the herdr-core terminal address (`t7`). The pane id form (`hc:t7`) also works.
`--cache` is a file the sidecar may read and write; the mod passes `<herdr-app home>/flockbook/cache.json`.

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

### `version`

```json
{"commands":["version","feed","status","card","approve","answer","send"],"type":"version"}
```

`commands`: the commands this sidecar has. The mod hides actions whose command is missing.

### `log` (`feed`)

```json
{"agent":"claude","cursor":"1.9c1d….0.48213.5be0…","model":"claude-opus-5-5","path":"/…/session.jsonl","size":48213,"start":0,"type":"log"}
```

| Field | Meaning |
|---|---|
| `agent` | The agent kind: `claude`, `codex`, `pi`, `cursor`, `grok`, … |
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

One per conversation row, in order. `id` is stable for the same log and `start`: the mod keys row
heights by it.

| `k` | Fields | Shown as |
|---|---|---|
| `user` | `text`, `ts`, `images` (count, when any) | A right-aligned bubble. |
| `assistant` | `text` (markdown), `ts` | Full-width markdown. |
| `tools` | `chips`, `ts` | A chip: "Ran 3 commands", expands to the calls. |
| `perm` | `perm`, `text` (the title), `ts` | The approval card (when `perm.state` is `pending`). |
| `question` | `question`, `ts` | The question card. |
| `status` | `text` | A centered note ("Context compacted"). |
| `sep` | `text` ("14:05", "Yesterday 09:12", "Mon 2 Nov 18:30") | A time separator. |
| `footer` | `text` ("01:41 · Worked 36s · 3 tools") | The turn footer, under the turn's last row. |

```json
{"id":"u1","k":"user","text":"Fix the login bug","ts":1793670000000,"type":"row"}
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

An approval card appears only when the screen shows the prompt ("Do you want …?" with "1. Yes").
A running tool is not an approval.

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
| `failed` | It did not go out. `message` says why. | Keeps the text or the card, shows `message`. |

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
