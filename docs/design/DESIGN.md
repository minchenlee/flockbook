# Flockbook design system

Date: 2026-10-08. Every r3 mock uses this file. The CSS tokens live in `:root` / `[data-theme="dark"]` of each mock (generated from one source, so they are identical).

## 1. Principles

1. **Gray first.** The UI is neutral gray. Color means something.
2. **One accent.** System blue = "you can act here" (primary button, selection, focus, links).
3. **Status colors are only for state.** Orange = needs you, green = done, blue arc = working, gray ring = idle. They never decorate.
4. **Shape + color.** Every state has its own shape, so it reads without color (HIG: "Avoid relying solely on color").
5. **Workspaces have no color.** A workspace is a gray monogram chip (`TK`, `JM`) + its name.

## 2. Color tokens

Swift: use the system color in the right column. Then light/dark, accent tint, and Increase Contrast come for free. The hex is only for the HTML mock.

### Neutrals

| Token | Light | Dark | Swift (macOS) | Use |
|---|---|---|---|---|
| `--bg` | `#FFFFFF` | `#1E1E1F` | `NSColor.textBackgroundColor` / `.windowBackgroundColor` | Main content area |
| `--sidebar` | `#F2F2F4` | `#262628` | `NSVisualEffectView` `.sidebar` material (SwiftUI `List(.sidebar)`) | Sidebar, tab bar strip |
| `--sidebar-sel` | `rgba(0,0,0,.07)` | `rgba(255,255,255,.09)` | system sidebar selection (`.selectedContentBackgroundColor` when unfocused) | Selected sidebar row |
| `--fill-1` | `rgba(0,0,0,.035)` | `rgba(255,255,255,.04)` | `.quaternarySystemFill` | Card insets, footers |
| `--fill-2` | `rgba(0,0,0,.06)` | `rgba(255,255,255,.07)` | `.tertiarySystemFill` | Chips, secondary buttons, user bubble |
| `--fill-3` | `rgba(0,0,0,.10)` | `rgba(255,255,255,.12)` | `.secondarySystemFill` | Monogram chip, progress track |
| `--sep` | `rgba(0,0,0,.09)` | `rgba(255,255,255,.09)` | `NSColor.separatorColor` | Hairlines |
| `--sep-strong` | `rgba(0,0,0,.14)` | `rgba(255,255,255,.15)` | `NSColor.gridColor` | Pane and sidebar edges |
| `--label` | `rgba(0,0,0,.85)` | `rgba(255,255,255,.88)` | `NSColor.labelColor` | Primary text |
| `--label-2` | `rgba(0,0,0,.50)` | `rgba(255,255,255,.55)` | `NSColor.secondaryLabelColor` | Secondary text, icons |
| `--label-3` | `rgba(0,0,0,.28)` | `rgba(255,255,255,.28)` | `NSColor.tertiaryLabelColor` | Times, placeholders, section labels |

### Accent (the only one)

| Token | Light | Dark | Swift | Use |
|---|---|---|---|---|
| `--accent` | `#007AFF` | `#0A84FF` | `NSColor.controlAccentColor` (follows the user's System Settings accent) | Primary button, selected list item, focus line, unread dot |
| `--accent-soft` | `#007AFF` 10% | `#0A84FF` 18% | `controlAccentColor.withAlphaComponent(0.1)` | Thumbnails, soft highlight |

### Status (state only)

| State | Token | Light | Dark | Swift | Shape |
|---|---|---|---|---|---|
| working | `--working` | `#007AFF` | `#0A84FF` | `NSColor.systemBlue` | gray ring + blue arc (spins) |
| blocked / needs you | `--blocked` | `#FF9500` | `#FF9F0A` | `NSColor.systemOrange` | filled circle + `!` |
| done | `--done` | `#28CD41` | `#32D74B` | `NSColor.systemGreen` | filled circle + check |
| idle | `--idle` | `rgba(0,0,0,.28)` | `rgba(255,255,255,.28)` | `NSColor.tertiaryLabelColor` | hollow circle |

Note: working uses blue, the same hue as the accent. This is on purpose: it keeps the palette to one hue plus orange and green. Working is told apart by its **arc shape**, not by hue. If the user picks a non-blue accent, working stays `systemBlue`.

Diff colors (`--diff-add` `systemGreen`, `--diff-del` `systemRed`) appear only inside diffs and the `+128 −12` stat. Red is also used for destructive menu items (`Close tab`).

## 3. Type (SF Pro)

Swift: use the text styles, not fixed sizes, so Dynamic Type and accessibility sizes work.

| Role | Size / weight | SwiftUI | Where |
|---|---|---|---|
| Large title | 26 / Bold | `.largeTitle.bold()` | Overview "Good evening" |
| Title | 22 / Bold | `.title` | Inbox detail agent name |
| Title 3 | 17 / Semibold | `.title3.weight(.semibold)` | List header, request title |
| Reading | 15 / Regular, line 1.55 | `.body` (15 at large size) | Conversation prose, user bubble |
| Headline | 13 / Semibold | `.headline` | Agent name, card title |
| Body | 13 / Regular | `.body` (macOS default) | Sidebar rows, menus, buttons |
| Callout | 12–12.5 / Medium | `.callout` | Chips, tabs, captions under composer |
| Caption | 11 / Medium–Semibold | `.caption` | Times, section labels, step labels |
| Mono | 12 / Regular, SF Mono | `.system(.body, design: .monospaced)` | Commands, file paths, code |

Numbers that change (times, counts, `+128`) use tabular figures (`.monospacedDigit()`).

## 4. Spacing, radius, elevation

- **Spacing (4-pt grid):** 4 · 8 · 12 · 16 · 24 · 32. Sidebar row height 28. Toolbar height 52. Pane header 46.
- **Radius:** 5 (monogram chip, menu item) · 6 (sidebar row, toolbar button) · 10 (menu, small card) · 14 (card, approval card) · 18 (composer). Pills use full height / 2.
- **Elevation:** only two levels.
  - `e1` = hairline + 1 px soft shadow. Cards that sit in the content.
  - `e2` = hairline + 8–10 px blur shadow. Things that float or need attention: approval card, composer, menu, popover.
  - Swift: `e1` = no shadow, `.background(.background, in: RoundedRectangle)` + separator stroke. `e2` = system `NSMenu` / `.popover` for menus; cards use `.shadow(radius: 14, y: 6)` at 12% opacity.
- **Materials:** the sidebar and tab strip are system materials (Liquid Glass on macOS 26). The content is opaque.

## 5. Icons

- **Real app: SF Symbols**, hierarchical rendering mode, `.imageScale(.medium)`, weight `.regular`.
- **HTML mocks: [Lucide](https://lucide.dev) v0.460.0**, ISC license ("Copyright (c) for portions of Lucide are held by Cole Bemis 2013-2022 as part of Feather (MIT). All other copyright (c) for Lucide are held by Lucide Contributors 2022."). Chosen because it is the closest open set to SF Symbols: rounded caps and joins, 24-grid, same metaphors. The SVGs are inlined as a `<symbol>` sprite.
- **One stroke:** 1.75 at all sizes. **One size scale:** 14 (inline, chips, sidebar disclosure) · 16 (default, rows, toolbar) · 20 (card icons).

| Purpose | Lucide (mock) | SF Symbol (Swift) |
|---|---|---|
| New tab | `square-pen` | `square.and.pencil` |
| Needs you | `inbox` | `tray` |
| Overview | `layout-grid` | `square.grid.2x2` |
| Search | `search` | `magnifyingglass` |
| Toggle sidebar | `panel-left` | `sidebar.left` |
| Add workspace | `folder` | `folder.badge.plus` |
| Filter | `list-filter` | `line.3.horizontal.decrease` |
| Add | `plus` | `plus` |
| Close | `x` | `xmark` |
| Terminal | `square-terminal` | `apple.terminal` |
| Read files | `file-text` | `doc.text` |
| Edit | `pencil` | `pencil` |
| Network | `globe` | `globe` |
| Dictation | `mic` | `mic` |
| Send | `arrow-up` | `arrow.up` (in `circle.fill`) |
| Disclosure | `chevron-right` / `chevron-down` | `chevron.right` / `chevron.down` |
| Branch | `git-branch` | `arrow.triangle.branch` |
| Phone preview | `smartphone` | `iphone` |
| Build | `hammer` | `hammer` |
| Check / done step | `check` | `checkmark` |
| Diff / Review | `file-diff` | `plusminus` (or `doc.badge.plus`) |
| Permission mode | `shield` | `shield` / `checkmark.shield` |
| More | `ellipsis` | `ellipsis` |
| Split right | `columns-2` | `rectangle.split.2x1` |
| Split down | `rows-2` | `rectangle.split.1x2` |
| Drag handle | `grip-vertical` | (system drag; no icon needed) |
| Role: orchestrator | `sparkles` | `sparkles` |
| Role: developer | `code` | `chevron.left.forwardslash.chevron.right` |
| Role: reviewer | `eye` | `eye` |
| Role: researcher | `book-open` | `book` |
| Role: designer | `pen-tool` | `pencil.and.outline` |
| Question | `circle-help` | `questionmark.circle` |
| Live test | `activity` | `waveform.path.ecg` |
| Rename | `text-cursor-input` | `character.cursor.ibeam` |
| Open externally | `external-link` | `arrow.up.forward.square` |
| Workspace / move to | `layers` | `square.stack` |

Agent role is inferred from the name suffix (`-orch`, `-dev`, `-rev`, `-res`, `-des`). Herdr does not store a role.

## 6. Status visuals

One shape and one color per state, the same everywhere (sidebar row, tab, avatar badge, card header, summary line).

| State | Small symbol (14 pt) | Ring around avatar | SF Symbol (Swift) |
|---|---|---|---|
| working | faint blue ring + blue quarter arc, 0.9 s a turn | blue arc = progress % | `SpinnerArc` (the same arc in the secondary text colour where something is waited for: `Spinner`) |
| blocked | orange disc + `!` | full orange ring | `exclamationmark.circle.fill` (orange) |
| done | green disc + check | none | `checkmark.circle.fill` (green) |
| idle | gray hollow circle | none | `circle` (tertiary label) |

Rules:
- Only **blocked** gets a count badge (sidebar "Needs you", Dock icon).
- Only **blocked** items get a primary (accent) button.
- Done and idle agents fold into one gray row ("4 done", "5 resting") with mini status dots.

## 7. Tabs model (r3.1 + r3.3: workspace sidebar + one top row with tabs)

Herdr: workspace → tabs → panes. Real snapshot (2026-10-08, read-only): 10 workspaces, 34 tabs, 35 panes, 33 agents. **33 of 34 tabs have exactly one pane; 32 of them hold one agent.** The only split tab is `Orchard / Orchestrator`: one agent pane on top, one plain shell pane below (split "down", ratio 0.5). One tab (`Studio / snapshot loop`) has one shell pane and no agent.

**Chosen model (owner, 2026-10-09; mock `04-ws-tabs.html` + `04-ws-tabs-dark.html`).** It replaces the round-3 recommendation (option B, `01b-tabs-sidebar.html`). Each level appears in exactly one place:

| Level | Where | Shows |
|---|---|---|
| Workspace | sidebar (one level only) | monogram chip + name + status summary |
| Tab | the top row (r3.3: merged with the toolbar) | the selected workspace's tabs only |
| Pane | main area | one conversation, or a split |

### Sidebar

- Top: New tab / Needs you (blocked badge) / Overview / Search. Unchanged.
- Then one row per workspace, height 34. No agent rows and no tab rows. No disclosure.
- Selected workspace: sidebar selection background + inverted monogram chip (`labelColor` fill).
- **Status summary**, right-aligned, always in this order:
  1. done / idle dots (6 pt; green filled = done, gray ring = idle), max 4, then `+n`;
  2. working arc + count, only if > 0;
  3. orange blocked badge with count, only if > 0. The badge is right-aligned, so badges line up in one column.
- A workspace with nothing blocked or working gets a secondary-label name (it reads as quiet).
- `+` in the "Workspaces" section header = new workspace.

### Top row (r3.3: one row; mock `06-one-row.html` + `06-one-row-dark.html`)

The toolbar, the tab bar and the pane header are merged into **one 52-pt glass row** (Safari compact-tab style). It replaces the separate 38-pt tab bar of 04/05.

Left to right:
1. **Tabs**, starting at the sidebar edge, in a **horizontally scrolling strip** (r3.5). Pill tabs, height 30, radius 8, **width = icon + title + padding** (r3.4; min 72, max 200, longer titles end in `…`). The selected tab gets the content color + `e1`. Unselected tabs are plain text with a hairline divider.
2. **All tabs** button (`chevron-down`; menu of every tab with state, ⌘1–9, a check on the current one, and "Show in Overview ⇧⌘\"), then `+` (new tab). Both stay pinned; they do not scroll.
3. Flexible space (also the window drag area).
4. **One set of actions:**
   - **Review**: file-diff icon + diffstat (`+18 −4`). Shown only when the tab's working tree has changes.
   - **Split**: one button with a menu (Split right ⌘D / Split down ⇧⌘D). Its icon shows the current layout.
   - **Show terminal** (⌃\`).

What was removed:
- The `Workspace · N tabs` breadcrumb. The selected sidebar row already names the workspace; the window title (Mission Control, Window menu) still is `Workspace — Tab`.
- The pane header for single-pane tabs. Agent name, state and age are in the **tab hover tooltip** (agent, state · activity · age, tab · pane count).
- The duplicate diff and terminal buttons inside the pane, and "Show terminal" under the composer. The composer keeps only the branch below it.

**Swift (r3.4 + r3.5):** `ScrollView(.horizontal, showsIndicators: false) { LazyHStack(spacing: 2) { ForEach(tabs) { TabPill($0).fixedSize().frame(minWidth: 72, maxWidth: 200) } } .scrollTargetLayout() }` with `.scrollPosition(id: $selectedTabID, anchor: .center)` so selection scrolls the tab into view, and `.mask(LinearGradient(...))` for the edge fades, only on edges that are clipped (read them with `onScrollGeometryChange`). The blocked edge marker is an overlay outside the mask; it is computed from which tab frames lie outside the visible rect.

**Split tabs only:** each pane gets a slim **28-pt label** (state + agent name + age, `×` on the right; shell pane: terminal icon + `Shell · zsh · <cwd>` + Open terminal). The focused pane's label has the accent underline. Single-pane tabs have no label.

### Tab rules

- Each tab: state symbol + tab label (the name set in Herdr). A **split tab** adds a split glyph (`rows-2` for down, `columns-2` for right). `×` shows on the selected tab and on hover.
- **Blocked tab:** orange `!` symbol + semibold label. Never truncated. If it is scrolled out of view, an **edge marker** (orange `!` + count, on a soft orange pill) sits at that edge of the strip; clicking it scrolls to the nearest blocked tab.
- **Too many tabs (r3.5, replaces "N more"):** nothing is hidden or reordered. The strip scrolls sideways (trackpad swipe, Shift + wheel). A clipped edge gets a soft fade (36 pt left, 44 pt right). Selecting a tab (click, ⌃Tab, ⌘1–9, the All tabs menu) scrolls it fully into view. A new tab is appended at the end and scrolled into view.
- `+` at the end of the bar = new tab in this workspace.

### Panes

- A split tab shows its panes in the Herdr layout direction (side by side for "right", stacked for "down"). The focused pane has an accent underline on its 28-pt label.
- An **agent pane**: conversation, composer, branch. No header when it is the only pane (r3.3).
- A **shell pane** (no agent): a compact terminal card. 28-pt label "Shell · zsh · <cwd>", the last 6 lines in SF Mono, "Open terminal", and `×`. No composer. It takes a fixed compact height (≈ 212) instead of the Herdr ratio, so the conversation keeps most of the space; "Open terminal" opens the full live terminal (`herdr terminal session observe|control`).

### Tab actions

| Action | Pointer | Keyboard | Menu |
|---|---|---|---|
| New tab | `+` after the tabs in the top row | ⌘T | File › New Tab |
| Next / previous tab | click | ⌃Tab / ⌃⇧Tab, ⌘1…⌘9 | Window menu |
| Next / previous workspace | click the sidebar row | ⌘⌥↓ / ⌘⌥↑ | — |
| Close tab | `×` on the selected / hovered tab | ⌘W | tab context menu (red) |
| Rename | double-click the tab label (inline edit) | — | tab context menu "Rename…" |
| Reorder | drag the tab; accent drop line shows the target | — | — |
| Move to workspace | drag the tab onto a sidebar workspace row | — | tab context menu "Move to workspace ›" |
| Split | Split button menu in the top row (right / down) | ⌘D / ⇧⌘D | tab context menu |
| Close pane | `×` in the 28-pt pane label (split tabs only) | ⇧⌘W | — |
| Jump to any tab | All tabs button (`⌄`) | ⌘1–9, ⌃Tab / ⌃⇧Tab | Window menu lists tabs |
| Scroll the tab row | trackpad swipe / Shift + wheel; edge marker jumps to a blocked tab | — | — |
| Agent details | hover the tab (tooltip) | — | — |

Earlier options (kept for reference): `01a-tabs-top.html` (A: sidebar workspaces → agents + tab bar; duplicated lists) and `01b-tabs-sidebar.html` (B: sidebar workspaces → tabs). `01-dark.html`, `02-overview.html`, `03-inbox.html` still use the B sidebar; under r3.1 their sidebar becomes the workspace-only list.

## 8. Components (what each mock reuses)

- **Sidebar row:** 28 high, 6 radius, state symbol + label + right-aligned time.
- **Workspace header:** disclosure chevron + monogram chip + name (semibold) + `+`.
- **Conversation:** user = right-aligned gray bubble; agent = plain 15-pt prose; work = step chips (`Read 6 files`, `Edited 4 files +128 −12`, `Ran 2 commands`); plan = card with progress bar and 5 icon steps.
- **Approval card (e2):** orange icon tile + one-line question + command chip + Deny (secondary) / Allow (accent) + "Always allow …" checkbox.
  - The chip shows the command. When the Bash call has a description, the chip shows it on a second line, in caption style, under the command.
  - No keyboard shortcuts and no shortcut hint. Allow, Deny and Always allow work by click only. (Same as the Swift app. The earlier `⌘↩ Allow · Esc Deny` hint is removed.)
  - **Mismatch state:** when the card cannot prove that the prompt on screen is this request, Deny and Allow are off and a caption in the accent orange says "The prompt on screen differs from this request — open the terminal". The card never sends a key in this state.
  - Edit, Write, WebFetch, MCP and other non-Bash calls, and parallel calls (more than one call still open), always use the mismatch state.
- **Composer (e2):** placeholder `Reply to <agent>`, `+`, model menu, shield, mic, round send button. Under it: branch + **Show terminal**.

## 9. Materials (r3.2: frosted glass)

Mocks: `05-glass.html`, `05-glass-dark.html`, `02b-overview.html`, `03b-inbox.html`.

Rule (HIG, Materials): **glass is for controls and navigation; content stays legible.** So the sidebar, bars and floating controls are translucent, and the conversation, cards and code stay almost opaque.

| Surface | Swift (macOS 26) | Fallback (macOS 14–15) | CSS stand-in (mock) |
|---|---|---|---|
| Window behind everything | system desktop, window shadow from AppKit | same | `--wall` soft gradient + window inset 18–24 px, radius 12, `--win-edge` shadow |
| Sidebar | `NavigationSplitView` + `List(.sidebar)` — Liquid Glass by default | `NSVisualEffectView.Material.sidebar`, `.behindWindow` | `--glass-side` (light `rgba(240,240,245,.55)`, dark `rgba(34,34,38,.55)`) + `backdrop-filter: blur(28px) saturate(180%)` |
| Toolbar | `.toolbar { }` on the window — system glass | `.titlebar` material | `--glass-bar` (light `rgba(248,248,251,.62)`, dark `rgba(38,38,42,.60)`) + same blur |
| Tabs (r3.3: inside the toolbar row) | tab pills as toolbar content (`ToolbarItem(placement: .principal)` or a custom title-bar accessory); the row is the system toolbar glass; selected tab = content color | `.titlebar` material | the `.toolbar` row is `--glass-bar` + blur; selected tab uses `--content` + `e1` |
| Needs-you list column (03b) | `List` in the content column with `.scrollContentBackground(.hidden)` + `.background(.thinMaterial)` | `.thinMaterial` | `--glass-bar` + blur |
| Composer | `.glassEffect(.regular, in: .rect(cornerRadius: 18))` | `.background(.regularMaterial, in: RoundedRectangle(cornerRadius: 18))` | `--glass-float` (light `rgba(255,255,255,.66)`, dark `rgba(48,48,52,.62)`) + blur + `e2` |
| Approval card | `.glassEffect(.regular.tint(.orange.opacity(0.08)), in: .rect(cornerRadius: 14))` | `.regularMaterial` | `--glass-float` + blur + `e2` |
| Menus, popovers, All tabs menu | `NSMenu` / `.popover` / `Menu` — system glass | system | `--glass-float` + blur |
| Conversation, cards, code, diffs | `.background(.background)` — opaque | same | `--content` (light `rgba(255,255,255,.92)`, dark `rgba(30,30,31,.90)`) |

Swift notes:
- Group nearby glass controls in one `GlassEffectContainer` (e.g. the toolbar buttons), so they blend instead of stacking.
- Do not put glass on glass (no glass card inside the glass sidebar). Cards inside glass get a plain fill (`--fill-1/2`).
- Respect **Reduce Transparency**: the system materials turn opaque by themselves; the custom tab pills must check `accessibilityReduceTransparency` and use `.background(.windowBackground)`.
- Use the system label colors on glass (`.foregroundStyle(.secondary)`). They are vibrant on materials, so contrast is handled by AppKit.

iced build notes (xplat: Linux, Windows, and macOS through iced):
- The window, the sidebar, the top row and the Needs-you list use the platform backdrop: macOS `NSVisualEffectView` (sidebar material), Windows 11 Mica, Linux opaque. The main area paints the almost opaque `--content` tint.
- The composer, the approval card, menus and tooltips stay opaque (e2). iced cannot blur the content under a float, and a thin tint over text is unreadable.
- Any backdrop failure, Reduce Transparency or an older Windows falls back to opaque. `--glass off` or `FLOCKBOOK_GLASS=off` turns glass off.

**Contrast on glass.** On glass, the mocks use stronger label tokens than on opaque surfaces:

| Token | Light on glass | Dark on glass |
|---|---|---|
| `--label-2` | `rgba(0,0,0,.62)` (was .50) | `rgba(255,255,255,.60)` (was .55) |
| `--label-3` | `rgba(0,0,0,.45)` (was .28) | `rgba(255,255,255,.42)` (was .28) |

Measured WCAG contrast, worst case over every wallpaper color, after the glass tint (script: blend glass over wallpaper, then text over that):

| Surface | label | label-2 | label-3 |
|---|---|---|---|
| light sidebar / bar / float | 12.0 / 12.7 / 13.3 | 5.6 / 5.8 / 5.9 | 3.2 / 3.2 / 3.3 |
| dark sidebar / bar / float | 11.0 / 10.9 / 10.1 | 6.0 / 5.9 / 5.6 | 3.7 / 3.7 / 3.6 |

- `label` and `label-2` pass WCAG AA (4.5:1) everywhere. Before the change, light `label-2` was 3.7–3.9 and failed.
- `label-3` passes 3:1. It is only for times, counts and placeholders. Never use it for text the user must read to act.
- The calculation treats blur as an average. A busy, high-contrast real wallpaper can be worse; this is why Swift should use the system vibrant label colors rather than these fixed values.
