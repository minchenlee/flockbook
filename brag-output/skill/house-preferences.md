
## House preferences

These come from the owner's feedback on the Flockbook launch video (2026-11-27). They refine the rules above; where they conflict, these win. Lines marked (interpretation) are how the feedback was applied, not the owner's own words.

**Product scenes**
- Do not build product scenes from screenshots. Animated crops of screenshots "do not look good". Rebuild the UI as live markup from the project's own values (theme colours, sizes, radii, icons, motion constants) and use screenshots only as the reference for layout and text.
- (interpretation) Keep one window and one camera across the product scenes: state changes inside the window (tabs, badges, cards, typing) and the camera pushes and pans to follow, instead of a hard cut per feature.
- (interpretation) Things happen for a reason on screen: a pointer clicks before a view changes, buttons show a pressed state, text streams in, lists fill row by row.

**Polish**
- The owner asked for "more refined". (interpretation) Use the project's own spring or easing constants when it has them, bring headlines in word by word, capture at 2× and scale down, and check zoomed stills for soft text (a 3D transform or CSS `scale` on a text layer blurs it; use `zoom` or re-lay it out).
- Everything on screen should have the finish the owner liked in the phone mock-up ("很有質感"). (interpretation) What gives it that finish: one light from above, so every raised surface has a thin lit top edge; surfaces are soft gradients, not flat fills; shadows come in layers (a tight contact shadow and a wide soft one); the subject sits in front of a dimmed, blurred background; glass gets a faint diagonal highlight; status colours glow a little. Apply this to windows, cards, buttons, terminals, the logo and the end card. Flat fills with a one-pixel border read as a diagram.
- The product's own UI is shown as the product draws it. Do not add shadows, gradients or highlights to buttons, message bubbles, cards or any other element inside the app window "because the original UI has none". Take each surface's fill, border and shadow from the source (here: flat theme fills; a small shadow only on the tab capsule and the reply box, a large one only on the Resources panel). (interpretation) Finish, depth and light belong to what surrounds the UI: the window's own drop shadow, the backdrop, the camera and its depth of field, device mock-ups, the logo and the end card.
- Keep the gloss low. A first pass with glass highlights, glows on status colours, a light sweep on the logo and strong gradient lettering was "a bit too much". Thin lit edges and layered shadows stay; sheens, glows and sweeps go.
- Film the UI like macro photography. (interpretation) Lay the window out large (CSS `zoom`, so text stays sharp), put it in perspective, and move a camera over it: very close, tilted shots of the detail that matters (a row of tool calls, a badge, a button, the reply line, a list), with a shallow depth of field (a sharp band at the focus line, blur growing above and below), slow drift, and a rack or crane move between subjects. Pull back between close shots so the viewer knows where they are. Text the viewer must read sits in the sharp band.
- The viewer must be able to follow each interaction: where the pointer goes, what it presses, what changes. The first macro cut was "good, but hard to see the operations". The owner offered two fixes: slow down and lengthen, or drop the camera work. (interpretation) Slow the product scenes (about 1.5×, still cut on the beat), let the video run to about 30 s, keep tilts small (15° or less) and keep both the control and its result in frame. Opening, logo and end card stay at normal speed.
- Do not add film grain at encode time without checking the file size: it made a 23 s file 242 MB.
- Logo and icon entrances move on one axis: parts rise straight up from below. No diagonal fly-ins.
- Device mock-ups must look like a real device: metal band, bezel, side buttons, island, status bar, glass highlight. A plain rounded rectangle is not enough.

**Sound**
- Effects come from the owner's sound pack, not from synthesis: Google Drive, folder "sounds" in My Drive (200 WAV files from React Bits; a `sounds.zip` sits beside it). Names describe the use: `tab`, `thock`, `keycap`, `chiclet`, `popover`, `check`, `dispatch`, `drawer`, `swoop`, `swish`, `emblem`, `arrive` and so on. Download only the ones the storyboard needs, measure each (length, loudest moment, dominant pitch), and place it so its loudest moment lands on the event. Do not commit the files to a public repository.
- Music: the owner picks a track on Pixabay. If a track file is supplied, use it and move the cuts onto its beat; otherwise synthesise a placeholder and say that it is one.
- The agent cannot hear. Say so when delivering, and report what was measured (levels, timing), not how it sounds.

**Content**
- Look at the repository's open pull requests for features that are about to ship and include the ones worth showing, labelled with their date ("Coming 12/1"). Do not show UI that a PR says has not been seen in a real window.
- On-screen copy is English, taken from the project's own README or site. Talk to the owner in Traditional Chinese.
