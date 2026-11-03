# Flockbook for herdr-app

Flockbook's reading view, rebuilt as a herdr-app layout and mod. It shows each coding agent as a
conversation instead of a terminal: a floating tab bar with one pill per agent, chat bubbles,
tool-call chips, a status tag row, and a composer.

It needs no change to herdr-app. Where herdr-app's mod API cannot do what Flockbook does, this
version comes close or leaves the feature out.

## Parts

| Path | What it is |
|---|---|
| `layouts/flockbook.json` | The window: a hidden sidebar and a stack of screens (chat, Needs you, Overview, the tab's panes). Each screen draws the top bar. |
| `mods/flockbook/` | One mod with several views. Each layout block picks one with `props.view`: `top`, `side`, `chat`, `needs`, `overview`; popups use `termbar` and `menu`. |
| `themes/` | Flockbook and Flockbook Mac themes, dark and light. |
| `test/` | Node tests with fake `gpui-kit`, `gpui-base` and `herdr` modules. |
| `keys.json` | The Flockbook shortcuts, in a `layout:flockbook` section (while this layout shows). Do not add a `mod:flockbook` section: with one, no key worked. |
| `install-isolated.sh` | Copies the layout, the mod, the keys and the sidecar into a test herdr-app home. |

The chat view reads conversations through a sidecar, `flockbook-modfeed`, built from this repo
(`swift build -c release --product flockbook-modfeed`). The mod finds it at
`<herdr-app home>/flockbook/bin/flockbook-modfeed`. Its protocol is in `docs/protocol.md`.

The layout and the mod work only together: the layout names the mod's views, and the mod expects
the layout's ids (`sidebar`, `screen`, `chat`, `tab`). The mod alone draws nothing (`place` is
`none`); the layout alone shows empty blocks where the views should be. The keys and themes are optional.

A mod cannot import files from another mod's folder, so all views live in one mod.

The sidebar column has the id `sidebar`, so herdr-app's own ⌘B (`layout.toggle:sidebar`) opens and closes it.

## Try it in a test home

Never install into your real herdr-app home while this is in development.

From a release: download `Flockbook-herdr-mod-<version>.zip` from GitHub Releases. It has this
folder's layout, mod, themes, keys and install script, and a universal, signed `flockbook-modfeed`
in `bin/`. Unzip it, then:

```sh
mkdir -p /tmp/flockbook-test-home
./install-isolated.sh /tmp/flockbook-test-home bin/flockbook-modfeed
```

From the repo:

```sh
mkdir -p /tmp/flockbook-test-home
./install-isolated.sh /tmp/flockbook-test-home path/to/flockbook-modfeed
HERDR_APP_HOME=/tmp/flockbook-test-home herdr-app check /tmp/flockbook-test-home/mods/flockbook
```

Then start herdr-app with `HERDR_APP_HOME=/tmp/flockbook-test-home` and pick the layout with
`layout.switch {"name": "flockbook"}`.

## Tests

```sh
npm test
```

## Release notes

Each change people can see adds one line to the repo's `docs/release-notes/next.md`.
