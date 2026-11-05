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
A machine whose helper is off in Flockbook settings (`Never`) shows its tabs, grouped by their labels, but
nothing is read or sent there, not even its layout.

Messages you typed while the agent works (its own queue: Claude Code, Codex, Grok Build) show faded at the end
of the conversation. Claude Code's can be pulled back to the composer, as on the Mac: Up, then Ctrl-U, never
Escape, and only text its log gives exactly. The helper keeps a pulled text until a page has it in its composer;
a page that was not connected gets it at its next connect. Codex's and Grok's are changed in their terminals.

Pictures in the conversation (ones you pasted, and screenshots a tool returned) load when they scroll into view.
The helper reads each one back from the log when the page asks, and checks that it is still the same picture.

Images you add in the composer (+) go to the helper with POST, part by part, then go out with your message as file
paths, as on the Mac. The helper keeps them in `<herdr-app home>/flockbook/uploads` (folder mode 700, files mode 600)
under names it makes, and removes only its own files: after 7 days, or the oldest past 200 files or 512 MB.

Notifications (the bell on the page) come from the helper itself, with the page open or closed: when an agent needs
you (a prompt to approve or answer), and when it is done after working. A message holds only the agent's title, its
name, the space and the tab, never conversation text. It is encrypted for your phone (Web Push, RFC 8291 and 8292), so
the push service sees only its size and time. The helper sends to the push services of Chrome and Android (FCM),
Safari and iOS (Apple), Firefox (Mozilla) and Edge (Windows) only, over HTTPS, and follows no redirect. A phone the
push service says is gone (404, 410) is removed. Like the Mac hub, one agent sends one kind at most every 2 minutes,
and at most 6 go out a minute; "needs you" always goes, and the rest are told of in one "N more agent updates".
The key and the phones' subscriptions are in `<herdr-app home>/flockbook/serve` (`vapid.key`, `subscriptions.json`,
mode 600, folder mode 700). When either file is there but cannot be read, the helper does not replace it: it says
`notifications off` in its log and the bell stays hidden. Delete the file to start again (every phone then turns
notifications on again).

A connection whose peer stops reading is closed after 5 s without progress on a message.

## Limits

- Any program on this machine, of any user, can connect to 127.0.0.1 (loopback is shared by all users)
  and send any header, the identity header included.
  - For your own programs, that is the trust they already have: they can run `hc` and send keys themselves.
  - Other users of the machine are not kept out. Do not run `serve` on a machine you share with users you do
    not trust.
- Pull back works on this machine's terminals only. On another machine the queue shows, but you change it in
  that terminal: a link that drops mid-pull could take the text from the agent and never bring it back.
- Scrolling back goes up to 2,000 older messages per conversation (or 32 MB of them); the Mac app shows the whole log.
- A pulled text that no page has taken yet is kept in memory only: it is gone when `serve` stops.
- Pictures go as they are in the log, PNG, JPEG and WebP only, up to 10 MB and 12,000 pixels a side.
  The Mac app re-encodes them (smaller previews, GIFs too); the helper does not.
  Others show an empty box. At most 300 pictures (or 256 MB) go out in 10 minutes, on all connections together.
- Pictures show for this machine's terminals only, not for terminals on another machine.
- Images you add are not re-encoded (the Mac app re-encodes them): PNG, JPEG and WebP up to 10 MB.
  Their metadata goes: EXIF and XMP (where a photo was taken, the camera), IPTC and comments, and PNG text chunks.
  A JPEG keeps only JFIF, its colour profile and its Orientation, so an upright photo still shows upright. What a phone
  puts after the image (a Motion Photo's video, a depth map or gain map) goes too. A file that cannot be read is refused.
  HEIC (an iPhone's own type) is refused: "This picture type cannot be sent; pick JPEG or PNG."
  At most 20 images (or 100 MB) in 10 minutes, and 8 waiting for a send.
- Images go to this machine's terminals only. A send with images to another machine is refused; nothing goes.
- Images waiting for a send are known in memory only: after `serve` restarts, add them again.
- Pull back takes what is queued in the pane now. If the pane runs another Claude Code session than the page shows,
  that session's text is pulled (the Mac hub does the same; the protocol says only the pane).
- A pulled text goes to one page at a time. When that page closes before it has the text in its composer, another
  page gets it at its next connect after 30 seconds, not sooner.
- A long pull (a queued text of many lines) keeps the request open for a while: about half a second per line.
- Notifications: the helper does not know one phone from another (no pairing). The bell shows on when it keeps any
  subscription, so a phone it removed (gone at the push service) can still show the bell on while another phone has
  one. Turn it off and on again on that phone. The Mac hub knows each device.
- Notifications are checked every 3 seconds: a prompt answered within that time may not be notified.
- Notifications are sent with `curl`. Without `curl` on the PATH none go out.

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
| `--home <folder>` | The herdr-app home whose Flockbook settings apply (`Never` for other machines). Default: `$HERDR_APP_HOME`, else the home this helper is installed in, else `~/.config/herdr-app`. |

## From the Herdr app mod (Flockbook settings)

The Flockbook screen in Herdr app has **Phone page** Off | On. On runs `flockbook-modfeed phone on`, which:

- reads your login and machine name from `tailscale status --json` (you can type another login),
- writes the same service file as `--print` below, with a first line that marks it as made by Flockbook
  settings, and the app's `HC_SESSION` / `HC_SOCKET`, and loads it (`launchctl bootstrap`, or
  `systemctl --user enable` and `restart`).

Off stops it and removes that one file, only when it has the marker. A file you installed by hand from
`--print` has no marker; the settings show it and leave it alone. The settings never run `tailscale serve`:
they show the command to copy, and once Serve is set, the address and a QR code. The port is config.json
`phone_port` (8787); Read-only is on the screen.

On Linux, a user service stops when you log out unless lingering is on: `loginctl enable-linger`.

## Run it at login by hand (not installed for you)

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
