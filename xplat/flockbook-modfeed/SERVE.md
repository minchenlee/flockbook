# `flockbook-modfeed serve`: the phone page from the helper

`serve` shows the Flockbook phone page (the same page the Mac app serves) from the Rust helper, on macOS
and Linux, without the Mac app. It reads agents and sends keys through herdr-core, the same way as the
mod's sidecar commands.

It is **off** until you start it. It installs nothing.

## How it is reached

- It listens on **127.0.0.1 only**. No option sets another address.
- Other devices reach it only through **Tailscale Serve**, which adds HTTPS and the caller's identity.
- It does no TLS and no pairing, and it never changes your Tailscale settings.

Every request must carry `Tailscale-User-Login`, and that login must be the account given with `--user`.
Otherwise the answer is 403. Serve removes any copy of that header that the caller sends.

Funnel is refused:

- Serve marks a Funnel request with `Tailscale-Funnel-Request` and sends no identity with it.
- So a Funnel request gets 403, even if you turn Funnel on by mistake.
- Do not use `tailscale funnel` for this port.

The socket (`/ws`) only reads.

Approve, answer and send go only with `POST /api`:

- The body must be JSON.
- The `Origin` must be the page's own `https://` origin.
- When the browser sends `Sec-Fetch-Site`, it must be `same-origin`. Older browsers do not send it, and then
  the Origin rule alone decides.
- A page from another site that is open in your browser therefore cannot use your Tailscale identity (CSRF).

`send` names the agent and the conversation the page showed. When another agent or another conversation is
in that pane now, nothing is sent (the same check as the mod's `send --expect-kind`, `--expect-path`).

Terminals on other machines (`hc machine add`) show in their own spaces, named and ordered by that machine's
Herdr layout, as in the Mac app. Their conversation, prompts and keys go through the helper on that machine,
which makes the same checks there. `serve` never installs that helper: install it from the mod's card.
A machine whose helper is off in Flockbook settings (`Never`) shows its tabs, but nothing is read or sent there.

Messages you typed while the agent works (its own queue: Claude Code, Codex, Grok Build) show faded at the end
of the conversation. Claude Code's can be pulled back to the composer, as on the Mac: Up, then Ctrl-U, never
Escape, and only text its log gives exactly. The helper keeps a pulled text until a page has it in its composer;
a page that was not connected gets it at its next connect. Codex's and Grok's are changed in their terminals.

A connection whose peer stops reading is closed after 5 s without progress on a message.

## Limits

- Any program on this machine, of any user, can connect to 127.0.0.1 (loopback is shared by all users)
  and send any header, the identity header included.
  - For your own programs, that is the trust they already have: they can run `hc` and send keys themselves.
  - Other users of the machine are not kept out. Do not run `serve` on a machine you share with users you do
    not trust.
- Pull back works on this machine's terminals only. On another machine the queue shows, but you change it in
  that terminal: a link that drops mid-pull could take the text from the agent and never bring it back.
- A pulled text that no page has taken yet is kept in memory only: it is gone when `serve` stops.
- Not in this version: pictures, image upload, and notifications.
  The helper answers those requests with "Not offered by this helper."

## Start it

```sh
# 1. The helper, as your user (herdr-core is found as the app finds it: HC_SOCKET, HC_SESSION).
flockbook-modfeed serve --user you@example.com            # port 8787

# 2. Tailscale Serve in front of it (HTTPS on your tailnet name). Not `tailscale funnel`.
tailscale serve --bg 8787
```

Open `https://<machine>.<tailnet>.ts.net/` on the phone.

`--user` is your login as `tailscale whois <phone's tailscale IP>` shows it.

Options:

| Option | Meaning |
|---|---|
| `--user <login>` | The one Tailscale account allowed. Needed. |
| `--port <n>` | The port on 127.0.0.1 (8787). |
| `--cache <file>` | The sidecar's cache and its `answered/` claims (`~/.flockbook/serve/cache.json`). |
| `--origin https://<host>` | An extra page origin to accept. Use it when a proxy changes `Host`. By default the `Origin` must name `Host` or `X-Forwarded-Host`. |
| `--read-only` | The page only reads. Approve, answer and send are refused. |

## Run it at login (not installed for you)

`--print` writes a service file for the options you give. Read the file before you install it.

macOS (launchd):

```sh
flockbook-modfeed serve --print launchd --user you@example.com > ~/Library/LaunchAgents/dev.flockbook.serve.plist
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/dev.flockbook.serve.plist
# log: ~/Library/Logs/flockbook-modfeed-serve.log
# stop: launchctl bootout gui/$(id -u)/dev.flockbook.serve
```

Linux (systemd, as your user):

```sh
mkdir -p ~/.config/systemd/user
flockbook-modfeed serve --print systemd --user you@example.com > ~/.config/systemd/user/flockbook-serve.service
systemctl --user daemon-reload && systemctl --user enable --now flockbook-serve
# log: journalctl --user -u flockbook-serve
# stop: systemctl --user disable --now flockbook-serve
```

`tailscale serve --bg` keeps its setting across restarts. `tailscale serve reset` removes it.
