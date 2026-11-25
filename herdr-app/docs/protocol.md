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
| `glance` | `[--cache <file>]` (Rust sidecar only) | one `glance` per agent on this machine |
| `summary` | `--terminal <addr> [--cache <file>]` (Rust sidecar only) | `summary`, only with the setting `summaries` on |
| `summary-models` | `--by <claude\|codex\|pi>` (Rust sidecar only) | `models` |
| `summary-usage` | `--cache <file> [--since <ms>]…` (Rust sidecar only) | one `summary-usage` per `--since` |
| `resources` | `--terminal <addr>` `[--since <cursor>]` `[--cache <file>]` (Rust sidecar only) | one `res` per resource; `end` has `cursor` and `full` |
| `open-check` | `--path <file>` (Rust sidecar only; this machine) | `openable` |
| `helper install` | `--machine <m> --sha256 <hex> [--cache <file>]` | `installed` |
| `helper remove` | `--machine <m> [--cache <file>]` | `removed` |
| `update-check` | `--home <herdr-app home>` `[--dismiss <x.y.z>]` (Rust sidecar only) | `update` |
| `phone status` / `on` / `off` | `--home <herdr-app home>` (Rust sidecar only) | `phone` |

`card`, `commands`, `glance`, `resources` and `open-check` only read. `approve` and `answer` send keys to the agent; the mod runs them only when the
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
{"commands":["version","feed","status","card","approve","answer","send","paste-image","helper","update-check","queue","phone","commands","glance","summary","summary-models","summary-usage","resources","open-check"],"release":"0.9.0","type":"version"}
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
{"agent":"claude","cursor":"2.9c1d….0.48213.5be0…","model":"claude-opus-5-5","path":"/…/session.jsonl","size":48213,"start":0,"type":"log"}
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
the SHA-256 of its base64. The mod never passes it.

One per conversation row, in order. `id` names the same message whatever range was read: the tail, a page
with `--before`, or Resources (which reads from the file's start). Claude ids come from the log; Codex, Grok and
Cursor ids from the message's line offset (`cx@1290`). The mod keys row heights by it.

| `k` | Fields | Shown as |
|---|---|---|
| `user` | `text`, `ts`, `images` (count, when any) | A right-aligned bubble. |
| | `pics` (with `--pictures` only) | Where the pictures are in the log, for `serve`. |
| | `from` (a note from another agent) | One line, "↳ docs-worker: first line", that opens to the whole note. |
| `assistant` | `text` (markdown), `ts`, `links` (when it has any) | Full-width markdown; the links can be clicked. |
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

`links[]` (assistant rows only, left out when there are none): what the agent points the human to in its reply, found by
the helper (`resources.rs`). Each: `start` and `end`, UTF-16 offsets into `text` (the text as sent, after the cut), and
`res`, the kind:

| `res` | What | Example |
|---|---|---|
| `doc` | An absolute path (`/…`, `~/…`) ending `.md .html .htm .pdf .png .svg` | `/home/me/plan.md` |
| `artifact` | A claude.ai artifact | `https://claude.ai/artifact/…`, also without `https://` |
| `pr` / `issue` / `ci` | A GitHub PR, issue or Actions run | `github.com/o/r/pull/12`, `/issues/7`, `/actions/runs/1870` |
| `local` | localhost, 127.0.0.1, a LAN address or `.local` host | `http://localhost:5173`, `localhost:3000` |
| `link` | Any other `http(s)` URL | `https://example.com/a` |

Found: bare URLs and paths, a Markdown link's address (once, not its label), an autolink `<…>`, and a code span that
holds only the target. The reply is read as CommonMark (pulldown-cmark, with tables);
only plain text is searched. Not found: user rows, tool output, code blocks, other text in code spans, pictures
(`![a](src)`), HTML (and the rest of a paragraph after an inline tag), reference links and definitions (`[a][1]`, `[1]: https://…`), a target that runs into an escape or
an entity (`\_`, `&amp;`), relative paths, paths with spaces. A target cut by the
`…` at the text cap is not a link. Punctuation at the end (`.` `,` `。`, a `)` without its `(`) is not part of a target.
The mod draws the web kinds as Markdown links in prose (not in code boxes or tables, nor on a line indented 4 or
more in its block: drawn alone, that block may show it as code; TextView opens them); `doc` stays
text for now. A mod that does not know `links` ignores it. After the helper is updated, a chat read before keeps its
rows for its `--cursor` (`unchanged`): their links show once the log changes and the rows come again in a `splice`.

```json
{"id":"a2","k":"assistant","links":[{"end":33,"res":"pr","start":4}],"text":"PR: https://github.com/o/r/pull/9 ok","ts":1793670007000,"type":"row"}
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

### `glance` (`glance`)

What each agent's last turn did, read from its log by rules. No model runs and nothing leaves the machine. One line
for every agent terminal on this machine, in herdr-core's order; a terminal on another machine gets none.

```json
{"agent":"claude","commands":3,"edited":5,"ended":1793670036000,"id":"a5-0","path":"/…/session.jsonl","said":"I found two ways.","terminal":"t7","tests":"passed","test":"pytest","type":"glance"}
```

The mod reads it for the chat's "Since you left" card: `id` to see whether the agent wrote more while the chat was
not on screen, and the counts for the card's line from the log ("Edited 5 files · pytest passed after the last
edit"). With the setting Summaries on, `said` and `ended` choose which agents' turns go to `summary`.

The last turn is everything after the newest user message in the last 2 MB of the log.

| Field | Meaning |
|---|---|
| `terminal`, `agent`, `path` | The terminal address, the agent kind, its log (`null`: none found; the counts are then 0). |
| `id` | The id of the turn's newest item. Another id means the agent wrote more. Absent: an empty log. |
| `ended` | When the turn's newest item came (ms since 1970). |
| `edited` | Files the edit tools changed in the turn, each once. An edit made by a shell command is not seen. |
| `commands` | Shell commands with a result. |
| `failed` | The newest finished command's first line, when it failed. Absent: it passed, or no command. Codex's log has no error flag: its exit code is read from the start of the output. |
| `tests` | The newest test command that finished after the last edit: `passed`, `failed`, or `ran` (its output went into a pipe without `pipefail`, so the exit code is not the tests'). `none`: a code file was edited and no test command finished after the last edit. Absent: no test command, and no edit but to files of words (`.md`, `.txt`, `.rst`, `.adoc`, `.org`, `.csv`, `.tsv`, `.log`). |
| `test` | That command's short name (`pytest`, `cargo test`). A test command is known by its program's name only: `pytest`, `tox`, `nox`, `unittest`, `jest`, `vitest`, `mocha`, `ava`, `rspec`, `phpunit`, `ctest`, `npm`/`yarn`/`pnpm`/`bun` `test`, `go test`, `cargo test`, `cargo nextest`, `swift test`, `deno test`, `dotnet test`, `node --test`, `playwright test`, `mvn test`, `gradle test`, `make`/`just` `test`, `xcodebuild test`; also after `sudo`, `env`, `npx`, `uv run`, `python -m`, `timeout <n>`, and after the `--` of a wrapper. |
| `said` | The first sentence of the turn's newest message, without markdown marks (at most 160 characters). In `said` a path that starts with `/` or `~/` is its last name only (`/w/src/a.rs` reads `a.rs`). |

`--cache`: where each agent's log is (as for `feed`). The lines are kept beside it in `glance.json` and reused
while a log's size and change time are the same, so a call is cheap when nothing was written. A line kept by a
helper that wrote other fields is not reused.

### `summary` (`summary`)

Two lines about an agent's last turn (the state of the work, and what it needs from the owner), written by a small model, for the chat's "Since you left" card. Off unless config.json has `"summaries": "on"`
(the mod's Flockbook settings, Summaries); while off the answer is the error `summaries-off`, nothing is read and
no program runs. The mod asks only for an agent that stopped and wrote something, once per turn, at most 3 at a
time (one call each, side by side), the newest first. It asks for every such agent on this machine, not only the one
whose chat is open, but only while a chat screen is open (the mod polls from the chat screen). Back from the Overview,
the card can show before its summary; the summary lines come in when the model answers.

```json
{"by":"claude","cached":false,"id":"a5-0","model":"claude-haiku-5-5","needs":"Say whether to update the changelog.","state":"The parser skips empty lines and the tests pass.","terminal":"t7","type":"summary"}
```

| Field | Meaning |
|---|---|
| `id` | The turn's newest item, as in `glance`. A summary is for that turn only. |
| `state` | One sentence: the state of the work (at most 240 characters). |
| `needs` | What the owner must do or decide now, or `null`. |
| `by` | The program that wrote it: `claude`, `codex` or `pi`. |
| `model` | The model that wrote it; for Codex and pi with none named, "Codex's default model" or "pi's default model". |
| `cached` | The kept answer for this turn (`summary.json` beside `--cache`); the model did not run. When a new answer is kept, the answers of terminals that are no agent on this machine any more are dropped. |

How it runs: one program, once, without a terminal (hc `machine.exec` `argv`, no shell, 60 s), in an empty folder
(`flockbook-summary` in the temp folder). Which program: config.json `"summary_by"` (Flockbook settings, Summary
written by): `own` (the default: the program of the agent that wrote the turn; Claude Code, Codex and pi agents), or
`claude`, `codex` or `pi` for the turns of every agent. `"summary_model"` (Summary model) names the model for a chosen
program: letters, digits and `._:/-`, at most 80, not led by `-`; anything else is not used. With `own` it is not used.

| Program | Arguments | Model when none is named |
|---|---|---|
| Claude Code | `claude -p --model <model> --no-session-persistence --tools "" --disable-slash-commands --strict-mcp-config --setting-sources "" --system-prompt <fixed text>` | `claude-haiku-5-5` |
| Codex | `codex exec --sandbox read-only --ephemeral --skip-git-repo-check --ignore-rules --ignore-user-config --color never -c model_reasoning_effort="low" --disable hooks --disable apps [-m <model>] -` | the one Codex uses by default |
| pi | `pi -p --no-session --no-tools --no-extensions --no-mcp --no-skills --no-prompt-templates --no-context-files --no-approve --no-themes --system-prompt <fixed text> [--model <model>]` | the one pi uses by default |

Claude Code and pi run with no tools, no saved session, no settings or context files, no MCP servers. (`claude
--bare` is not used: it reads no login, so an account without an API key would fail. Hooks of installed plugins may
still start.) Codex cannot run without its tools: it runs in a sandbox that only reads, with no rules, no config
file, no hooks and no session file, and it has no system prompt option, so the fixed text leads its stdin.
The stdin: the owner's message that started the last turn (at most 1 500 characters), the end of what the agent wrote in that
turn (at most 6 000), and one line of the `glance` facts. With `own` the text goes to the provider that agent's
conversation already goes to, on that agent's account, and nowhere else; with one program chosen it goes to that
program's provider, also for the turns of other agents.
The answer must have a line `STATE: …`; `NEEDS: …` is optional and `NEEDS: nothing` means none. Other
lines are dropped. A kept answer is for one turn, one program and one model: after a change of either the turn is
asked again, and so is an answer kept by a helper that asked for other lines.

Each run of a model adds one line to `summary-usage.jsonl` beside `--cache`: the time, the program, the model, the
characters that went (the stdin, and the fixed text when it goes apart) and came, the time it took, and whether it
worked. No word of the turn or of the answer is in it. A kept answer is not a run.

`summary-usage --cache <file> [--since <ms>]…`: one line for each `--since` (0 with none), from that count:

```json
{"failed":1,"got":5149,"ms":412000,"runs":42,"sent":96321,"since":1793635200000,"type":"summary-usage"}
```

`runs`: the runs of a model from `since` on; `failed`: those that gave no summary; `sent`, `got`: characters, not
tokens (the programs do not all say their tokens); `ms`: the time they took together. The command only reads the
count. When the file is over 512 KB, lines older than 35 days are dropped (the file is written whole through a temp file).

Errors: `summaries-off`; `summary-unsupported` (with `own`, an agent whose program writes none; or a terminal on another machine);
`summary-none` (no conversation, or the agent wrote nothing in its last turn); `summary-failed` (the program is
missing, failed, ran out of time, or answered in another shape; `message` says which). A failure is not kept, so the
next call tries again.

### `models` (`summary-models`)

The models a program can run for a summary, for the list in the mod's settings. Nothing of an agent is read or sent.

```json
{"by":"pi","models":[{"id":"opencode-go/glm-5.3-flash","note":"1M context"}],"source":"program","type":"models"}
```

| `source` | Where the list is from |
|---|---|
| `built-in` | Claude Code: it has no command that lists models, so a short list is kept in the helper. |
| `file` | Codex: `models_cache.json` in its home (`CODEX_HOME`, else `~/.codex`), the models it shows in its picker. No program runs. |
| `program` | pi: `pi --offline --list-models`, run once without a terminal (hc `machine.exec`, 20 s). |

`id` is what `summary_model` takes. A name that is not in the list can still be set. Errors: `usage` (no such
program), `models-failed` (pi did not run).

The "Since you left" card shows the two lines under "Summary · written by a model, may be wrong", above the line
from the log, never in its place. After any error it shows the line from the log alone. No other screen shows them.

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

### `res` (`resources`)

What the agent pointed the human to in this conversation: one line per resource, newest first (by the last mention; in
one reply, the later target first). Read only.

```json
{"type":"res","id":"9c1e…","kind":"doc","target":"/home/user/notes/plan.md","title":"The plan","detail":"~/notes","row":"a3-0","first_ts":1793670005000,"last_ts":1793670020000,"mentions":2,"exists":true}
```

- Collected with the same rules as a row's `links` (assistant text only), and the claude.ai artifact URL in the result of
  a successful Artifact publish (its `title` from the call's input). Not collected: what the person typed, other
  tools' input and output, relative paths.
- Commands (`cmd`, assistant text only): a fenced code block whose language (the info's first word, any case) is
  none, `sh`, `bash`, `zsh`, `shell` or `console`, holding one command: one line, or lines joined by a `\` at
  their end; a leading `$ ` is dropped; at most 400 characters. Not commands: a block with more lines (a script, a
  command and its output), other languages (`text`, `log`, `json`…), indented code, inline code, a comment.
  `target`: the command as written (its lines kept), trimmed.
- `kind`: `doc`, `artifact`, `pr`, `issue`, `ci`, `local`, `link`, `cmd`. A mod ignores kinds it does not know.
- `id`: FNV-1a (hex) of the normalized target; the same target is one item. A doc: its absolute path (`~/` is the
  home of the machine the agent runs on). `local`: one per host:port, `target` the newest URL. PR, issue, CI run: their
  GitHub page (`/pull/12/files` is `/pull/12`). Other URLs: host lowercase, without `#…` and a trailing `/`. A
  command: the command trimmed (key `cmd:<command>`).
- `title`: a doc's first `# ` heading (Markdown) or `<title>` (HTML) in its first 4 KB, else its file name; an
  artifact's title (from the publish call, else its page's `<title>` when that file is here); `o/r#12`, `CI run 1870`, `localhost:5173`, host and path; a command's: the last line of the paragraph or heading right
  before its block (an ending `:` or `：` dropped, at most 80 characters), else the command's start (60). `detail`: a
  doc's folder (`~` for the home), a URL's host, or `o/r`; a command on one line (each line's ending `\` dropped).
- `row`: the `id` of the feed `row` with the last mention. `first_ts`, `last_ts`: ms, or null.
- `exists`: docs only; the path's real file (links followed) is a file now, and the real file's extension is a doc kind
  (`md`, `html`, `htm`, `pdf`, `png`, `svg`). A `preview.md` link to `notes.txt` or to a folder: `false`. It can be old
  by the time the person clicks: the mod asks `open-check` before it opens a file.
- The whole log is counted, at any length. `row` can be older than the rows the chat has loaded: then "Show in chat"
  does nothing.
- With `--cache`, the helper keeps what it parsed in `resources/<hash of the terminal>.json` beside it (one file per
  terminal, written through a temp file and renamed; at most 64 files, the newest kept, and files not changed for 7
  days are dropped; a file that does not parse counts as none). Next time it parses only the lines after a safe start and gives
  the same answer as a whole read; a log that is shorter, changed at its start or another file is read whole. Without
  `--cache` the whole log is read each time.
- `end` has `cursor` (null when the agent has no log) and `full`. Pass the cursor back as `--since`: the same log,
  unchanged, gives no `res`; the same log, grown, gives only the resources mentioned after the cursor's newest one
  (`full` false: merge them by `id`). Another log, a shorter one or a bad cursor gives them all (`full` true: replace).

```json
{"type":"end","ok":true,"ms":4,"cursor":"r2.af63…ce1.48213.1793670020000","full":false}
```

### `openable` (`open-check`)

The real file a doc path opens, looked at now: links followed (`canonicalize`), the real path must be a file, and the
real file's extension (lowercase) a doc kind (`md`, `html`, `htm`, `pdf`, `png`, `svg`). Read only; on this machine.

```json
{"type":"openable","ok":true,"path":"/home/user/notes/b.md"}
{"type":"openable","ok":false,"why":"Flockbook does not open this kind of file."}
```

- `path`: the real file. The mod opens this path, not the one it asked about.
- `why`: `File not found.`, `Not a file.`, `Flockbook does not open this kind of file.` or `The file name is not plain text.`
  (a real path that is not UTF-8, on Linux: it is not sent, so the mod never opens another path than the one checked), for
  the person to read.
- A helper without `open-check` (its `version` does not list it): the mod opens no file.

### `commands` (`commands`)

The "/" commands of an agent's terminal, for the list above the reply box: Claude Code, Codex, pi, agy, omp and
grok. Read from files only; no key goes to the agent. Another agent: `commands` is empty (the mod shows no list).

```json
{"type":"commands","agent":"claude","commands":[{"name":"clear","description":"Start a new session with empty context; previous session stays on disk (resumable with /resume)","source":"builtin","hint":"[name]"},{"name":"deploy","description":"Deploy the app","source":"project"},{"name":"model","description":"Set the AI model for Claude Code","source":"builtin","hint":"<model>","args":[{"value":"claude-fable-5-2","label":"Fable 5.2"},{"value":"claude-opus-5-5","label":"Opus 5.5"},{"value":"opus","label":"The latest Opus"}],"screen":true}],"screens":["diff","help","hooks","memory","permissions","plugin","release-notes","rewind","skills","status","tasks","usage"]}
```

- Sources per agent (`agent`), the first one with a name wins:
  - `claude`: built-ins (the ones people use most that the chat can send: `add-dir`, `clear`, `compact`, `context`,
    `effort`, `export`, `init`, `model`, `resume`, `code-review`); `.claude/commands/**/*.md` in the terminal's folder (`project`), then in the user's
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
  - `omp`: its built-ins only. Its custom commands and skills come from many providers turned on in its settings, so
    they are not read.
  - `grok`: its built-ins; skills (`.grok/skills/<name>/SKILL.md` and `.agents/skills/<name>/SKILL.md`, one level; not
    when its front matter says `user-invocable: false`) and commands (`.grok/commands/*.md`, `.agents/commands/*.md`,
    top level; `project` up to the repository root, `user` in the home), from the terminal's folder up to the
    repository root, then in the home (the nearest wins); skills have the source `skill`. Not read: the Claude and
    Cursor folders and `[skills]` folders of its `config.toml`.
- `description`: the front matter's `description`, else the file's first line; at most 120 characters.
- Each when it says something: `hint`, what may follow the name (`<model>`, `[file]`; for a file, its front matter's
  `argument-hint`); built-ins only: `args`, the values the mod offers after `/<name> `, each `{"value", "label"}` (no
  `label` where the value says it all; the mod also takes plain strings). Fixed lists in the helper: Claude Code's, agy's,
  omp's and grok's `effort`, pi's `thinking`. `model`, per agent (models.rs), where its `/model <value>` switches with no screen:
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
  - `omp`, `grok`: no value list. omp's `/model` is in `screens`; grok's has a hint and `screen: true`.
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
