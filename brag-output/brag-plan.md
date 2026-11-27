# Flockbook brag plan

**What it is:** a reading view for Herdr coding agents: read what each agent did, see which one needs you, reply in one place.
**Angle:** agent terminals are full of noise; Flockbook turns them into a page you can read.
**Tone:** polished, calm, dark (the app's own grey theme, Inter + JetBrains Mono, amber "needs you" dot).
**Format:** landscape 1920×1080, 30 fps, 24 s. Music 120 BPM (beat = 0.5 s), every cut on a beat.

| Time | Scene | On screen |
|---|---|---|
| 0–4 s | Hook | Six agent terminals scroll. "Agent terminals are full of noise." → one pane asks "Do you want to proceed?" → "Which one needs you?" |
| 4–7 s | Reveal | The icon assembles. "Flockbook. A reading view for your Herdr coding agents." |
| 7–10.5 s | Read | One live app window rises in. The agent's turn streams in: tool chips, calls, then the answer word by word. "Read what each agent did." |
| 10.5–12 s | Needs you | The camera moves to the sidebar: the Needs you badge counts 1 → 2 → 3, status dots turn amber. The pointer clicks the waiting tab. "See which one needs you." |
| 12–14 s | Approve | The approval card springs up; the pointer presses Allow once; the card folds away and the agent runs. "Answer without opening the terminal." |
| 14–16.5 s | Reply | A message is typed and sent; the bubble rises into the thread and the answer streams in. "Reply in one place." |
| 16.5–19 s | Resources (open PRs #24, #29) | The pointer switches workspace and opens Resources; the panel slides in and its rows fill. "Everything the session touched." Tag: Resources · Coming 12/1 |
| 19–20.5 s | Phone page | The window falls back and blurs; a phone rises with the Needs you cards. "And from your phone." |
| 20.5–24 s | Outro | The icon's arrowheads fly into a V. "Your whole flock, on one page." github.com/minchenlee/flockbook · Free · MIT · macOS & Linux · Unofficial, not affiliated with Herdr |

**Source material:** the app window is rebuilt in HTML from the mod's own values: colours from `herdr-app/themes/flockbook.json`, sizes, radii and the spring from `herdr-app/mods/flockbook/*.js`, icons from `mods/flockbook/icons`. The mod draws with a native toolkit, so its components cannot run in a browser; the preview screenshots were the reference for layout and for all text. The phone uses the tokens in `web/style.css`. All data is the repo's demo data (shop, atlas, fbdemo-coop); the terminal noise in the hook is illustrative. One camera moves over one window from 7 s to 20.5 s; frames are captured at 2× and scaled down.
**Sound and timing:** music is "Cascade Breathe" (future garage, 130 BPM) from Pixabay, chosen by the owner; the file is not in the repo. The composition's timeline is written at 120 BPM. `work/timemap.json` maps video beats to that timeline: opening, logo and end card at normal speed, the product scenes about 1.5× slower so the interactions can be followed, every scene change on a beat of the track. The video is 30.5 s. `work/mix.py` takes the track from 103.60 s (bar 56) so the end card lands on the track's own ending, muffles the first two bars and opens the filter on the logo beat, and places the sound-pack samples through the same map. `work/synth.py` is the earlier synthesised placeholder.

**Camera:** the window is laid out at 2.5× and filmed in perspective with a focus band (depth of field), close on the detail in use, with small tilts.

Times in the table above are on the 120 BPM timeline.
